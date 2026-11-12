"""Collect what the bot builds, buys and is offered on turns 1 to 12.

Measurement for docs/briefs/late-buys.md. It plays the fixed eval's games at
lag 0 (one learner seat, seven real seats, frozen boards, the real lobby
filter) and writes one JSON row per game. Nothing here trains or changes the
engine; it only reads bgpy observations and `env.seats`.

    .venv/bin/python train/evaltools/late_buys/collect_bot.py \
        --ckpt train/runs/r1111-lagmix/best.pt --name r1111-lagmix --games 2048 \
        --vram-gb 10 --out train/runs/late-buys/raw-r1111-lagmix.json

Offers come from the Tavern slots in the observation: a slot counts a new
offer when its card id changes. Buys are the `Buy(slot)` actions the network
takes, so they are exact and do not guess from hand or shop differences.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from collections import Counter
from pathlib import Path

import numpy as np
import torch

ROOT = Path(__file__).resolve().parents[2]
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
TARGETS = tuple(range(1, 41))
import os
LAG = int(os.environ.get("LAG", "0"))
LATE = (9, 10, 11, 12)
# The action index of a Tavern Refresh, derived from the flat action layout
# (Move, Roll, Freeze, Level, End Turn).
ROLL = ppo.END_TURN - 3


def parse_args():
    a = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    a.add_argument("--ckpt", required=True)
    a.add_argument("--name", required=True)
    a.add_argument("--games", type=int, default=2048)
    a.add_argument("--seed-base", type=int, default=SEED_BASE)
    a.add_argument("--boards", type=Path, default=FROZEN)
    a.add_argument("--vram-gb", type=float, required=True)
    a.add_argument("--out", type=Path, required=True)
    return a.parse_args()


def slots(row, start: int, n: int) -> list[tuple[int, bool, int, int, bool] | None]:
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


def board(row) -> list[dict]:
    out = []
    for i, m in enumerate(slots(row, bgpy.BOARD_START, bgpy.BOARD)):
        if m is None:
            continue
        card, _, attack, health, golden = m
        out.append({
            "card": card,
            "position": i + 1,
            "golden": golden,
            "attack": attack,
            "health": health,
        })
    return out


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
                          real_lag=LAG, real_lag_from=7)
        if info["overlay_comps"]:
            env.set_overlay(info["overlay_comps"], 0.0)
        if names is None:
            names = env.card_names()
        b = Bufs(n)
        env.kinds(b.kinds)
        done = np.zeros(n, dtype=bool)
        rec = [None] * n
        # Per game, per turn.
        boards = [[None for _ in TARGETS] for _ in range(n)]
        tiers = [[None for _ in TARGETS] for _ in range(n)]
        offers = [dict() for _ in range(n)]
        offer_spell = [dict() for _ in range(n)]
        buys = [dict() for _ in range(n)]
        buy_spell = [dict() for _ in range(n)]
        late_buys = [dict() for _ in range(n)]
        prev_shop = [None] * n
        prev_turn = [None] * n
        rolled = [False] * n
        ar = np.arange(n)
        torch.manual_seed(args.seed_base + chunk)
        steps = 0
        while not done.all():
            env.observe(b.obs, b.mask, b.seat, b.kind)
            turns = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
            current = (~done) & (b.kind == LEARNER)
            shops = [None] * n
            for e in np.nonzero(current)[0]:
                e = int(e)
                row = b.obs[e]
                turn = int(turns[e])
                if not 1 <= turn <= 40:
                    prev_shop[e] = None
                    prev_turn[e] = turn
                    continue
                shop = slots(row, bgpy.SHOP_START, bgpy.SHOP)
                shops[e] = shop
                # The Tavern compacts when a card is bought, so compare the
                # multiset of cards, not slot indices. A Roll deals a fresh
                # Tavern and every card on it is a new offer.
                curr = Counter(m[0] for m in shop if m is not None)
                fresh = (prev_turn[e] != turn or prev_shop[e] is None
                         or rolled[e])
                if fresh:
                    for m in shop:
                        if m is None:
                            continue
                        name = names[m[0]]
                        offers[e][name] = offers[e].get(name, 0) + 1
                        offer_spell[e][name] = offer_spell[e].get(name, False) or m[1]
                else:
                    for cid, count in curr.items():
                        extra = count - prev_shop[e].get(cid, 0)
                        if extra <= 0:
                            continue
                        spell = any(m is not None and m[0] == cid and m[1]
                                    for m in shop)
                        name = names[cid]
                        offers[e][name] = offers[e].get(name, 0) + extra
                        offer_spell[e][name] = offer_spell[e].get(name, False) or spell
                prev_shop[e] = curr
                prev_turn[e] = turn
                if turn in TARGETS:
                    k = TARGETS.index(turn)
                    boards[e][k] = [
                        {"name": names[m[0]], "card": m[0], "golden": m[4],
                         "attack": m[2], "health": m[3], "position": i + 1}
                        for i, m in enumerate(slots(row, bgpy.BOARD_START, bgpy.BOARD))
                        if m is not None
                    ]
                    tiers[e][k] = int(row[bgpy.SCALAR_TIER])

            o = torch.from_numpy(b.obs).to(dev)
            mask = force_mask(o, torch.from_numpy(b.mask).to(dev))
            comp = None
            if net.n_comps:
                env.overlay_probe(b.forced1, b.flags)
                comp_idx = b.flags[ar, b.seat, F_COMP] - 1
                comp = comp_onehot(comp_idx, net.n_comps, dev)
            acts, _, _, _ = act(net, o, mask, comp=comp)
            acts_np = acts.cpu().numpy()

            for e in np.nonzero(current)[0]:
                e = int(e)
                turn = int(turns[e])
                a = int(acts_np[e])
                rolled[e] = (a == ROLL)
                if not 1 <= turn <= 40 or not (0 <= a < bgpy.SHOP):
                    continue
                if shops[e] is None:
                    continue
                m = shops[e][a]
                if m is None:
                    continue
                name = names[m[0]]
                buys[e][name] = buys[e].get(name, 0) + 1
                buy_spell[e][name] = buy_spell[e].get(name, False) or m[1]
                if turn in LATE:
                    late_buys[e][name] = late_buys[e].get(name, 0) + 1

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
                    "boards": {str(t): boards[e][TARGETS.index(t)]
                               for t in TARGETS if boards[e][TARGETS.index(t)] is not None},
                    "tiers": {str(t): tiers[e][TARGETS.index(t)]
                              for t in TARGETS if tiers[e][TARGETS.index(t)] is not None},
                    "offers": dict(sorted(offers[e].items())),
                    "offer_spell": dict(sorted(offer_spell[e].items())),
                    "buys": dict(sorted(buys[e].items())),
                    "buy_spell": dict(sorted(buy_spell[e].items())),
                    "late_buys": dict(sorted(late_buys[e].items())),
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
        "real_lag": 0,
        "real_lag_from": 1,
        "elapsed_seconds": round(time.time() - started, 1),
        "rows": rows,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload) + "\n")
    print(f"wrote {args.out} ({len(rows)} games, {payload['elapsed_seconds']}s)",
          flush=True)


if __name__ == "__main__":
    main()
