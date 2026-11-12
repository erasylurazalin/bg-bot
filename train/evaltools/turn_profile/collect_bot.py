"""Collect the lag-mix bot's shop actions on turns 7 to 12.

Measurement for docs/briefs/turn-profile.md. It plays the fixed eval's games
(one learner seat, seven real seats, frozen boards, the real lobby filter) and
writes one JSON row per game with per-turn counts. Nothing here trains or
changes the engine; it only reads bgpy observations.

    .venv/bin/python train/evaltools/turn_profile/collect_bot.py \
        --ckpt train/runs/r1111-lagmix/best.pt --real-lag 0 --real-lag-from 1 \
        --games 2048 --vram-gb 10 --out train/runs/turn-profile/raw-lag0.json

The games are the fixed eval's games: chunk k is `VecEnv(n, SEED_BASE + k)`
and each env plays its first game to the end. Actions are sampled like
`train/fixed_eval.py`. The action kind is read from the flat action index
(`bgpy.ACTION_KINDS`, the same list as `ppo.KINDS`).

What one turn row holds (see `docs/briefs/turn-profile.md` for the brief):

- `actions`: every env action the learner took in the turn, End Turn and the
  action that hits the 200-action cap included.
- `actions_grouped`: the same minus `target`, `target_shop` and `discard`,
  which Era's logged steps fold into the buy, play or activate that opened
  them. `choose` stays its own action because Era logs a Discover as a step.
- `kinds`: raw counts by action kind.
- `rolls`, `buys`, `sells`, `cycles`, `spells_bought`, `spells_played`,
  `hero_powers`, `level_gold`.
- `tier_end`, `gold_end` before the End Turn action (or before the action
  that hits the cap).
- `ended_by_cap`: the action was not End Turn, `actions_left` was 1 before it,
  and the next observation was a later turn or the game ended. That is the
  engine's own condition in `finish_shop` (`actions >= 200`), except the
  Lobster search, which the turn-change test leaves out.
- `gold_end_exact`: false when the turn ended on the cap and the last action's
  Gold price could not be read exactly off the observation.

A buy-then-sell cycle is matched by card id: the learner bought a minion and
then sold a board minion of the same card id in the same turn. The observation
has no entity ids, so two copies of one card are indistinguishable.
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
TARGETS = tuple(range(7, 13))
CONTINUATION = {"target", "target_shop", "discard"}

# Flat action offsets, from the same table the model uses.
_OFFSET = {}
_o = 0
for _name, _size in bgpy.ACTION_KINDS:
    _OFFSET[_name] = _o
    _o += _size
PLAY_START = _OFFSET["play"]
SELL_START = _OFFSET["sell"]
# Play { hand, pos } is hand * (BOARD + 1) + pos.
PLAY_WIDTH = bgpy.BOARD + 1
SHOP_SLOT_COST = 37
SHOP_SLOT_GOLDEN = 7


def parse_args():
    a = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    a.add_argument("--ckpt", required=True)
    a.add_argument("--real-lag", type=int, default=0)
    a.add_argument("--real-lag-from", type=int, default=1)
    a.add_argument("--games", type=int, default=2048)
    a.add_argument("--seed-base", type=int, default=SEED_BASE)
    a.add_argument("--boards", type=Path, default=FROZEN)
    a.add_argument("--vram-gb", type=float, required=True)
    a.add_argument("--out", type=Path, required=True)
    return a.parse_args()


def slots(row, start: int, n: int) -> list[dict | None]:
    """The card slots at `start`, as the observation writes them."""
    out = []
    for i in range(n):
        b = start + i * bgpy.SLOT_FEATURES
        if row[b] <= 0.5:
            out.append(None)
            continue
        out.append({
            "id": int(row[b + 1]) - 1,
            "spell": bool(row[b + 2] > 0.5),
            "attack": int(row[b + 3]),
            "health": int(row[b + 4]),
            "golden": bool(row[b + 7] > 0.5),
            "cost": int(row[b + SHOP_SLOT_COST]),
        })
    return out


def new_turn(turn: int) -> dict:
    return {
        "turn": turn,
        "actions": 0,
        "actions_grouped": 0,
        "kinds": {},
        "rolls": 0,
        "buys": 0,
        "sells": 0,
        "cycles": 0,
        "spells_bought": 0,
        "spells_played": 0,
        "hero_powers": 0,
        "level_gold": 0,
        "tier_end": None,
        "gold_end": None,
        "gold_end_exact": True,
        "ended_by_cap": False,
        "ended_by_end": False,
        "bought": [],
        "last": None,
    }


def start_turn(state: dict, row) -> None:
    state["tier_start"] = int(row[bgpy.SCALAR_TIER])
    state["gold_start"] = int(row[bgpy.SCALAR_GOLD])


def end_price(row, kind: str, a: int) -> int | None:
    """Gold the last action takes, when the observation can give it exactly."""
    if kind in ("target", "target_shop", "choose", "discard", "move",
                "freeze", "end", "choice", "trinket", "dark_gift",
                "hero_power", "hero_power2"):
        if kind in ("hero_power", "hero_power2"):
            base = bgpy.SCALAR_POWER_COST if kind == "hero_power" else bgpy.SCALAR_POWER2_COST
            return int(row[base])
        return 0
    if kind == "roll":
        # A free Refresh costs nothing; a Health Refresh costs no Gold.
        if row[bgpy.SCALAR_FREE_ROLLS] > 0.5:
            return 0
        if row[bgpy.SCALAR_HEALTH_REFRESHES_LEFT] > 0.5:
            return 0
        if row[bgpy.SCALAR_NEXT_ROLL_HEALTH] > 0.5:
            return 0
        return 1
    if kind == "level":
        return int(row[bgpy.SCALAR_UPGRADE_COST])
    if kind == "buy_spell":
        return int(row[bgpy.SCALAR_SHOP_SPELL_COST])
    if kind == "buy":
        shop = slots(row, bgpy.SHOP_START, bgpy.SHOP)
        slot = a - _OFFSET["buy"]
        if 0 <= slot < bgpy.SHOP and shop[slot] is not None:
            return int(shop[slot]["cost"])
    return None


def close_turn(st: dict) -> None:
    """Fill the end-of-turn fields from the last action's pre-state."""
    if st["ended_by_end"]:
        return
    last = st["last"]
    if last is None:
        st["gold_end_exact"] = False
        return
    st["tier_end"] = last["tier"]
    if last["kind"] == "end":
        st["ended_by_end"] = True
        st["gold_end"] = last["gold"]
        return
    if last["actions_left"] <= 1:
        # The engine's finish_shop condition: actions >= 200. A Lobster
        # search owns the turn end and a turn change is then not seen here.
        st["ended_by_cap"] = True
        cost = end_price(last["row"], last["kind"], last["action"])
        if cost is None:
            st["gold_end_exact"] = False
        else:
            st["gold_end"] = last["gold"] - cost
    else:
        st["gold_end_exact"] = False


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
          f"comps {info['n_comps']}, lag {args.real_lag} from turn {args.real_lag_from}",
          flush=True)

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
        # Per env, per turn of interest, and all-turn cap counts.
        turns = [dict() for _ in range(n)]
        all_turns = [0] * n
        cap_turns = [[] for _ in range(n)]
        state = [None] * n
        ar = np.arange(n)
        torch.manual_seed(args.seed_base + chunk)
        steps = 0
        while not done.all():
            env.observe(b.obs, b.mask, b.seat, b.kind)
            obs_turn = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
            current = (~done) & (b.kind == LEARNER)
            for e in np.nonzero(current)[0]:
                e = int(e)
                t = int(obs_turn[e])
                st = state[e]
                if st is not None and st["turn"] != t:
                    close_turn(st)
                    all_turns[e] += 1
                    if st["ended_by_cap"]:
                        cap_turns[e].append(st["turn"])
                    if 7 <= st["turn"] <= 12:
                        turns[e][str(st["turn"])] = public_turn(st)
                    state[e] = None
                    st = None
                if st is None:
                    st = new_turn(t)
                    start_turn(st, b.obs[e])
                    state[e] = st
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
                t = int(obs_turn[e])
                st = state[e]
                if st is None or st["turn"] != t:
                    continue
                a = int(acts_np[e])
                kind = ppo.KINDS[int(ppo.KIND_OF[a])]
                row = b.obs[e]
                shop = slots(row, bgpy.SHOP_START, bgpy.SHOP)
                hand = slots(row, bgpy.HAND_START, bgpy.HAND)
                board = slots(row, bgpy.BOARD_START, bgpy.BOARD)
                st["actions"] += 1
                st["kinds"][kind] = st["kinds"].get(kind, 0) + 1
                if kind not in CONTINUATION:
                    st["actions_grouped"] += 1
                if kind == "roll":
                    st["rolls"] += 1
                elif kind == "buy":
                    st["buys"] += 1
                    slot = a - _OFFSET["buy"]
                    if 0 <= slot < len(shop) and shop[slot] is not None:
                        st["bought"].append({
                            "id": shop[slot]["id"],
                            "golden": shop[slot]["golden"],
                        })
                elif kind == "buy_spell":
                    st["spells_bought"] += 1
                elif kind == "sell":
                    st["sells"] += 1
                    slot = a - SELL_START
                    if 0 <= slot < len(board) and board[slot] is not None:
                        m = board[slot]
                        for i, got in enumerate(st["bought"]):
                            if got["id"] != m["id"]:
                                continue
                            # Prefer a copy with the same golden flag.
                            if got["golden"] != m["golden"]:
                                continue
                            del st["bought"][i]
                            st["cycles"] += 1
                            break
                        else:
                            for i, got in enumerate(st["bought"]):
                                if got["id"] == m["id"]:
                                    del st["bought"][i]
                                    st["cycles"] += 1
                                    break
                elif kind == "play":
                    hi = (a - PLAY_START) // PLAY_WIDTH
                    if 0 <= hi < len(hand) and hand[hi] is not None and hand[hi]["spell"]:
                        st["spells_played"] += 1
                elif kind in ("hero_power", "hero_power2"):
                    st["hero_powers"] += 1
                elif kind == "level":
                    st["level_gold"] += int(row[bgpy.SCALAR_UPGRADE_COST])

                st["last"] = {
                    "kind": kind,
                    "action": a,
                    "actions_left": int(row[bgpy.SCALAR_ACTIONS_LEFT]),
                    "gold": int(row[bgpy.SCALAR_GOLD]),
                    "tier": int(row[bgpy.SCALAR_TIER]),
                    "row": row.copy(),
                }
                if kind == "end":
                    st["ended_by_end"] = True
                    st["gold_end"] = int(row[bgpy.SCALAR_GOLD])
                    st["tier_end"] = int(row[bgpy.SCALAR_TIER])

            kinds = b.kinds.copy()
            env.step(acts_np, b.places, b.tier_turns, b.tribes, b.reset)
            for e, s in zip(*np.nonzero(b.places)):
                e, s = int(e), int(s)
                if kinds[e, s] != LEARNER or done[e]:
                    continue
                place = int(b.places[e, s])
                st = state[e]
                if st is not None:
                    close_turn(st)
                    all_turns[e] += 1
                    if st["ended_by_cap"]:
                        cap_turns[e].append(st["turn"])
                    if 7 <= st["turn"] <= 12:
                        turns[e][str(st["turn"])] = public_turn(st)
                    state[e] = None
                rec[e] = {
                    "game": start + e,
                    "chunk": chunk,
                    "env": e,
                    "place": place,
                    "knocked_out_turn": None if place == 1 else int(obs_turn[e]),
                    "turns": turns[e],
                    "turns_played": all_turns[e],
                    "cap_turns": cap_turns[e],
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
        "force_dark_gift": info["force_dark_gift"],
        "old_boards": info["old_boards"],
        "obs_size": bgpy.OBS_SIZE,
        "n_actions": bgpy.N_ACTIONS,
        "name": "r1111-lagmix",
        "games": args.games,
        "seed_base": args.seed_base,
        "chunk": CHUNK,
        "lineup": LINEUP,
        "boards": str(args.boards),
        "lobby_filter": True,
        "real_lag": args.real_lag,
        "real_lag_from": args.real_lag_from,
        "targets": list(TARGETS),
        "elapsed_seconds": round(time.time() - started, 1),
        "rows": rows,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload) + "\n")
    print(f"wrote {args.out} ({len(rows)} games, {payload['elapsed_seconds']}s)",
          flush=True)


def public_turn(st: dict) -> dict:
    return {
        "actions": st["actions"],
        "actions_grouped": st["actions_grouped"],
        "kinds": dict(sorted(st["kinds"].items())),
        "rolls": st["rolls"],
        "buys": st["buys"],
        "sells": st["sells"],
        "cycles": st["cycles"],
        "spells_bought": st["spells_bought"],
        "spells_played": st["spells_played"],
        "hero_powers": st["hero_powers"],
        "level_gold": st["level_gold"],
        "tier_start": st["tier_start"],
        "tier_end": st["tier_end"],
        "gold_start": st["gold_start"],
        "gold_end": st["gold_end"],
        "gold_end_exact": st["gold_end_exact"],
        "ended_by_cap": st["ended_by_cap"],
        "ended_by_end": st["ended_by_end"],
    }


if __name__ == "__main__":
    main()
