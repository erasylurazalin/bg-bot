"""Does swapping one minion for Leeroy or Deadly Spore change a late fight?

For docs/briefs/swap-check.md. Plays the same fixed eval as the scam check
(2,048 games per lag, the same seeds, lineup, frozen boards and lobby filter)
and saves, for every learner turn from 10 on at lag 0 and at lag 2 from turn
7, the board that fought and the opponent it fought, in the board-file JSON
shape `bgpy.BoardFight` takes. Ghost fights are left out. The fight's actual
result is resolved from the learner's next observation (won/lost flags, both
false for a tie) or from the placement when the fight knocked it out.

Then each pair is fought 100 times per arm with the same seed for every arm:
baseline, Leeroy / Spore in the weakest minion's slot, Spore in its best slot,
a control that puts a plain copy of the median minion in the weakest slot,
Leeroy / Spore for the strongest minion in the best slot, and an add variant
on boards with room. The baseline is checked against the actual results
before anything else; the analysis stops if the shares miss by more than 2
points.

    .venv/bin/python train/evaltools/swap_check.py collect --vram-gb 10
    .venv/bin/python train/evaltools/swap_check.py analyze

Writes train/runs/swap-check/pairs.json (collect), then sanity.json,
rows.json and report.md (analyze). Measurement only.
"""

from __future__ import annotations

import argparse
import json
import math
import multiprocessing as mp
import re
import statistics
import sys
import time
import tomllib
from datetime import datetime, timezone
from pathlib import Path

import numpy as np
import torch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "train"))
sys.path.insert(0, str(ROOT / "train" / "evaltools"))

import bgpy
import fixed_eval
import ppo
import real_boards
from ppo import Bufs, F_COMP, LEARNER, comp_onehot, force_mask
from scam_check import board_from_seat, board_from_slots, opponent_block

CKPT = ROOT / "train" / "runs" / "r1111-lagmix" / "best.pt"
OUT = ROOT / "train" / "runs" / "swap-check"
REF = ROOT / "ref" / "bg-cards.json"
FIRST_TURN = 10
FIGHTS = 100
KEEP_POWER_TRINKETS = False
LEEROY_NAME = "Leeroy the Reckless"
SPORE_NAME = "Deadly Spore"
LEEROY_TIER = 5
SPORE_TIER = 3
LEEROY = {"name": LEEROY_NAME, "attack": 6, "health": 2,
          "golden": False, "keywords": []}
SPORE = {"name": SPORE_NAME, "attack": 1, "health": 1,
         "golden": False, "keywords": ["venomous"]}
GAPS = (
    ("under 0.1", 0.0, 0.1),
    ("0.1 to 0.25", 0.1, 0.25),
    ("0.25 to 0.5", 0.25, 0.5),
    ("0.5 to 1", 0.5, 1.0),
    ("over 1", 1.0, math.inf),
)
TURNS = (("10-11", 10, 11), ("12-13", 12, 13), ("14+", 14, 10 ** 6))
WON = bgpy.SCALAR_WON_LAST_COMBAT
LOST = bgpy.SCALAR_LOST_LAST_COMBAT


def parse_args():
    a = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = a.add_subparsers(dest="stage", required=True)

    c = sub.add_parser("collect", help="play the games and save the pairs")
    c.add_argument("--ckpt", type=Path, default=CKPT)
    c.add_argument("--games", type=int, default=2048)
    c.add_argument("--lags", default="0,2",
                   help="comma-separated real-seat lags (default 0,2)")
    c.add_argument("--lag2-from", type=int, default=7,
                   help="first turn a nonzero lag applies on (default 7)")
    c.add_argument("--seed-base", type=int, default=fixed_eval.SEED_BASE)
    c.add_argument("--boards", type=Path, default=fixed_eval.FROZEN)
    c.add_argument("--vram-gb", type=float,
                   help="cap this process's GPU memory; required when CUDA is used")
    c.add_argument("--cpu", action="store_true", help="run on the CPU")
    c.add_argument("--out-dir", type=Path, default=OUT)

    a_ = sub.add_parser("analyze", help="sanity check, arms, report")
    a_.add_argument("--pairs", type=Path, default=OUT / "pairs.json")
    a_.add_argument("--out-dir", type=Path, default=OUT)
    a_.add_argument("--jobs", type=int, default=8,
                    help="worker processes for the fights (default 8)")
    a_.add_argument("--force", action="store_true",
                    help="continue even when the sanity check misses by over 2 points")
    a_.add_argument("--with-power-trinkets", action="store_true",
                    help="fight the hero power and trinkets the observation exposes too")
    return a.parse_args()


# ---- card ids -------------------------------------------------------------


def ref_ids() -> dict[str, str]:
    """Engine card name to a ref card id that builds it: the base id for a
    name, plus the ids `real_boards` renames (Dark Paradox, Floating Candle)."""
    raw = json.loads(REF.read_text())
    ids: dict[str, str] = {}
    for c in raw["cards"]:
        if c.get("is_golden"):
            continue
        ids.setdefault(c["name"], c["card_id"])
    ids["Floating Candle"] = "BG36_MagicItem_208t"
    for suffix in ("t3", "t4", "t5", "t6", "t9"):
        ids[f"Dark Paradox {suffix}"] = f"BG36_360{suffix}"
    return ids


def card_id(ids: dict[str, str], name: str) -> str:
    # A token the ref data has no card for (Cobra, Golem): an id no source
    # knows makes a Stand-in with the row's exact stats and keywords, which is
    # what a board file with an unknown token does.
    return ids.get(name, "UNKNOWN:" + name)


def minion_json(m, ids) -> dict:
    return {
        "card_id": card_id(ids, m["name"]),
        "name": m["name"],
        "attack": int(m["attack"]),
        "health": int(m["health"]),
        "golden": bool(m["golden"]),
        "keywords": list(m["keywords"]),
    }


def own_counters(row) -> dict:
    return {
        "gem_attack": int(row[bgpy.SCALAR_GEM_ATTACK]),
        "gem_health": int(row[bgpy.SCALAR_GEM_HEALTH]),
        "spell_attack": int(row[bgpy.SCALAR_SPELL_ATTACK]),
        "spell_health": int(row[bgpy.SCALAR_SPELL_HEALTH]),
    }


def opp_counters(row, block) -> dict:
    return {
        "gem_attack": int(row[block + bgpy.OPP_GEM_ATTACK]),
        "gem_health": int(row[block + bgpy.OPP_GEM_HEALTH]),
        "spell_attack": int(row[block + bgpy.OPP_SPELL_ATTACK]),
        "spell_health": int(row[block + bgpy.OPP_SPELL_HEALTH]),
    }


def board_json(turn, tier, minions, counters, power=None, trinkets=None) -> dict:
    board = {
        "turn": int(turn),
        "tier": int(tier),
        "minions": minions,
        "counters": counters,
    }
    if power:
        board["hero_power"] = power
    if trinkets:
        board["trinkets"] = trinkets
    return board


def power_and_trinket_tables():
    """Starting-power card ids by hero name, standalone powers by power name,
    and trinkets in table order as (name, card id, slot)."""
    heroes = tomllib.loads((ROOT / "heroes.toml").read_text())
    power_ids: dict[str, str] = {}
    for h in heroes.get("hero", []):
        p = h.get("power")
        if p:
            power_ids[h["name"]] = p["card_id"]
            power_ids.setdefault(p["name"], p["card_id"])
    for p in heroes.get("power", []):
        power_ids.setdefault(p["name"], p["card_id"])
    # trinkets.toml has multi-line inline tables, which TOML 1.0 does not
    # allow; read the three keys a board needs block by block.
    trinkets: list[tuple[str, str, str]] = []
    text = (ROOT / "trinkets.toml").read_text()
    for block in re.split(r"(?m)^\[\[trinket\]\]$", text)[1:]:
        name = re.search(r'^name = "([^"]+)"', block, re.M)
        card = re.search(r'^card_id = "([^"]+)"', block, re.M)
        slot = re.search(r'^slot = "([^"]+)"', block, re.M)
        if name and card:
            trinkets.append((name.group(1), card.group(1),
                             slot.group(1) if slot else "lesser"))
    return power_ids, trinkets


def own_power(row, hero_names, power_ids) -> dict | None:
    i = int(row[bgpy.SCALAR_POWER_HERO])
    if i <= 0 or i > len(hero_names):
        return None
    name = hero_names[i - 1]
    cid = power_ids.get(name)
    if not cid:
        return None
    # Onyxia's Broodmother is the one fight power whose magnitude is a number.
    numbers = ([int(row[bgpy.SCALAR_POWER_PROGRESS])]
               if cid == "BG22_HERO_305p" else [])
    return {"card_id": cid, "name": name, "numbers": numbers}


def side_trinkets(row, base, trinkets) -> list[dict]:
    out = []
    for slot in range(bgpy.TRINKET_SLOTS):
        i = int(row[base + slot])
        if i <= 0 or i > len(trinkets):
            continue
        name, cid, slot_kind = trinkets[i - 1]
        out.append({"card_id": cid, "name": name,
                    "greater": slot_kind == "greater", "numbers": []})
    return out


# ---- collect --------------------------------------------------------------


def block_for(learner_seat: int, opp_seat: int) -> int | None:
    """The opponent block of `opp_seat` in a learner observation, or None."""
    k = (opp_seat - learner_seat) % bgpy.PLAYERS
    if k == 0:
        return None
    return bgpy.OWN_END + (k - 1) * bgpy.OPP_BLOCK


def last_opponent_block(o, learner_seat: int) -> tuple[int | None, int | None]:
    """The opponent block the observation marks as the last one fought."""
    for k in range(1, bgpy.PLAYERS):
        block = bgpy.OWN_END + (k - 1) * bgpy.OPP_BLOCK
        if o[block + bgpy.OPP_LAST] > 0.5:
            return (learner_seat + k) % bgpy.PLAYERS, block
    return None, None


def make_pair(game, lag, turn, seats, row, names, learner_seat, ids,
              hero_names, power_ids, trinkets) -> dict | None:
    opp_seat, block, opp_alive = opponent_block(row, learner_seat)
    if opp_seat is None or row[bgpy.SCALAR_GHOST] > 0.5 or not opp_alive:
        return None
    board = [minion_json(m, ids) for m in board_from_seat(seats, learner_seat)]
    opp_board = [minion_json(m, ids) for m in board_from_seat(seats, opp_seat)]
    tier = int(row[bgpy.SCALAR_TIER])
    opp_tier = int(row[block + bgpy.OPP_TIER])
    power = own_power(row, hero_names, power_ids)
    own_trinkets = side_trinkets(row, bgpy.SCALAR_TRINKET, trinkets)
    opp_trinkets = side_trinkets(row, block + bgpy.OPP_TRINKET, trinkets)
    return {
        "game": int(game),
        "lag": int(lag),
        "turn": int(turn),
        "learner_seat": int(learner_seat),
        "opp_seat": int(opp_seat),
        "tier": tier,
        "opp_tier": opp_tier,
        "board": board_json(turn, tier, board, own_counters(row),
                            power=power, trinkets=own_trinkets),
        "opp_board": board_json(turn, opp_tier, opp_board,
                                opp_counters(row, block),
                                trinkets=opp_trinkets),
        "opp_board_source": "env.seats",
        "actual": None,
    }


def collect_lag(args, info, real, lag: int, lag_from: int, dev, ids,
                power_ids, trinkets) -> list[dict]:
    net = info["net"]
    pairs: list[dict] = []
    started = time.time()
    chunk = 0
    drops = {"cap": 0, "unresolved": 0, "ghost": 0}
    while chunk * fixed_eval.CHUNK < args.games:
        start = chunk * fixed_eval.CHUNK
        n = min(fixed_eval.CHUNK, args.games - start)
        env = bgpy.VecEnv(n, args.seed_base + chunk, fixed_eval.LINEUP,
                          real_boards=real, old_boards=info["old_boards"],
                          real_lobby_filter=True, real_lag=lag,
                          real_lag_from=lag_from)
        if info["overlay_comps"]:
            env.set_overlay(info["overlay_comps"], 0.0)
        names = env.card_names()
        hero_names = env.hero_names()
        b = Bufs(n)
        env.kinds(b.kinds)
        done = np.zeros(n, dtype=bool)
        learner_seat = [-1] * n
        pending: list[dict | None] = [None] * n
        torch.manual_seed(args.seed_base + chunk)
        steps = 0
        while not done.all():
            env.observe(b.obs, b.mask, b.seat, b.kind)
            turns = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
            left = b.obs[:, bgpy.SCALAR_ACTIONS_LEFT].astype(np.int64)
            current = (~done) & (b.kind == LEARNER)
            # Resolve the last turn's fight from this turn's flags.
            for e in np.nonzero(current)[0]:
                e = int(e)
                learner_seat[e] = int(b.seat[e])
                p = pending[e]
                if p is None:
                    continue
                if p["ended_by"] == "end_turn":
                    o = b.obs[e]
                    p["actual"] = ("win" if o[WON] > 0.5
                                   else ("loss" if o[LOST] > 0.5 else "tie"))
                    # The seen block holds the opponent board that went into
                    # the fight, after its last shop and before combat. The
                    # End Turn snapshot can predate that shop.
                    seat, block = last_opponent_block(o, int(b.seat[e]))
                    if block is not None:
                        seen = [minion_json(m, ids) for m in board_from_slots(
                            o, block + bgpy.OPP_SCALARS, names)]
                        if seen:
                            p["opp_seat"] = seat
                            p["opp_board"] = board_json(
                                p["turn"], p["opp_tier"], seen,
                                opp_counters(o, block),
                                trinkets=p["opp_board"].get("trinkets"))
                            p["opp_board_source"] = "observation_last_seen"
                        fb = block
                        if o[fb + bgpy.OPP_FIGHT_HAPPENED] > 0.5:
                            p["actual_opp"] = int(round(
                                o[fb + bgpy.OPP_FIGHT_OUTCOME]))
                    p["actual_won"] = bool(o[WON] > 0.5)
                    p["actual_lost"] = bool(o[LOST] > 0.5)
                    p["actual_last_damage"] = int(o[bgpy.SCALAR_LAST_DAMAGE])
                    pairs.append(p)
                else:
                    drops["cap"] += 1
                pending[e] = None
            o = torch.from_numpy(b.obs).to(dev)
            mask = force_mask(o, torch.from_numpy(b.mask).to(dev))
            comp = None
            if net.n_comps:
                env.overlay_probe(b.forced1, b.flags)
                comp_idx = b.flags[np.arange(n), b.seat, F_COMP] - 1
                comp = comp_onehot(comp_idx, net.n_comps, dev)
            acts = fixed_eval.sample_actions(net, o, mask, comp, 1.0)
            acts_np = acts.cpu().numpy()
            ends = (acts_np == ppo.END_TURN) | (left <= 1)
            for e in np.nonzero(current & ends)[0]:
                e = int(e)
                turn = int(turns[e])
                if turn < FIRST_TURN:
                    continue
                row = make_pair(start + e, lag, turn, env.seats(e),
                                b.obs[e], names, int(b.seat[e]), ids,
                                hero_names, power_ids, trinkets)
                if row is None:
                    drops["ghost"] += 1
                    continue
                row["ended_by"] = ("end_turn" if int(acts_np[e]) == ppo.END_TURN
                                   else "cap")
                pending[e] = row

            # A learner who ended the turn early still has opponents shopping
            # after it. Keep the pending row's opponent board at the latest
            # pre-combat snapshot, so a fight that knocks the learner out is
            # not compared with a board from before those shops.
            for e in np.nonzero((~done) & (b.kind != LEARNER))[0]:
                e = int(e)
                p_row = pending[e]
                if p_row is None:
                    continue
                seats = env.seats(e)
                opp = p_row["opp_seat"]
                if opp >= len(seats):
                    continue
                seen = [minion_json(m, ids) for m in board_from_seat(seats, opp)]
                if seen:
                    p_row["opp_board"] = board_json(
                        p_row["turn"], p_row["opp_tier"], seen,
                        p_row["opp_board"]["counters"],
                        trinkets=p_row["opp_board"].get("trinkets"))
                    p_row["opp_board_source"] = "latest_snapshot"

            env.step(acts_np, b.places, b.tier_turns, b.tribes, b.reset)
            need_all = False
            for e, s in zip(*np.nonzero(b.places)):
                e, s = int(e), int(s)
                if (s == learner_seat[e] and pending[e] is not None
                        and not b.reset[e]
                        and pending[e]["ended_by"] == "end_turn"):
                    need_all = True
                    break
            allobs = live = None
            if need_all:
                allobs = np.zeros((n, bgpy.PLAYERS, bgpy.OBS_SIZE), np.float32)
                live = np.zeros((n, bgpy.PLAYERS), bool)
                env.observe_all(allobs, live)
            for e, s in zip(*np.nonzero(b.places)):
                e, s = int(e), int(s)
                if s != learner_seat[e] or done[e]:
                    continue
                p = pending[e]
                if p is not None:
                    if p["ended_by"] == "end_turn":
                        # The dead learner's own observation, when the game
                        # goes on, still carries the fight's result and the
                        # board that fought. A reset game has neither.
                        if allobs is not None and not b.reset[e]:
                            o = allobs[e, s]
                            p["actual"] = ("win" if o[WON] > 0.5
                                           else ("loss" if o[LOST] > 0.5
                                                 else "loss"))
                            p["resolved_by"] = "dead_observation"
                            seat, block = last_opponent_block(o, s)
                            if block is not None:
                                seen = [minion_json(m, ids) for m in board_from_slots(
                                    o, block + bgpy.OPP_SCALARS, names)]
                                if seen:
                                    p["opp_seat"] = seat
                                    p["opp_board"] = board_json(
                                        p["turn"], p["opp_tier"], seen,
                                        opp_counters(o, block),
                                        trinkets=p["opp_board"].get("trinkets"))
                                    p["opp_board_source"] = "dead_observation"
                        else:
                            # A reset game keeps no observation: place 1 is
                            # the game win, which is also a won fight.
                            p["actual"] = ("win" if int(b.places[e, s]) == 1
                                           else "loss")
                            p["resolved_by"] = "reset_place"
                        pairs.append(p)
                    else:
                        drops["cap"] += 1
                    pending[e] = None
                learner_seat[e] = -1
            for e in np.nonzero(b.reset)[0]:
                e = int(e)
                if pending[e] is not None:
                    drops["unresolved"] += 1
                    pending[e] = None
                learner_seat[e] = -1
                done[e] = True
            steps += 1
            if steps % 2000 == 0 and not done.all():
                print(f"    lag {lag} chunk {chunk}: {int(done.sum())}/{n} done, "
                      f"{len(pairs)} pairs, {time.time() - started:.0f}s", flush=True)
        print(f"  lag {lag} chunk {chunk} done: {int(done.sum())}/{n} games, "
              f"{len(pairs)} pairs total, {steps} steps, "
              f"{time.time() - started:.0f}s", flush=True)
        chunk += 1
    pairs.sort(key=lambda r: (r["game"], r["turn"]))
    print(f"  lag {lag}: dropped {drops['ghost']} ghost or empty opponent rows, "
          f"{drops['cap']} cap rows, {drops['unresolved']} unresolved", flush=True)
    return pairs


def collect(args):
    if args.games < 1:
        raise SystemExit("--games must be at least 1")
    lags = [int(x) for x in args.lags.split(",") if x.strip()]
    folder = args.boards.resolve()
    n_board_files = fixed_eval.check_boards(folder)
    real, unknown = real_boards.load(folder)
    if not real:
        raise SystemExit(f"{folder}: no real boards")
    print(f"real boards from {folder}: {n_board_files} files, {len(real)} boards"
          + (f", {len(set(unknown))} unmatched names" if unknown else ""), flush=True)

    dev = torch.device("cpu" if args.cpu else
                       ("cuda" if torch.cuda.is_available() else "cpu"))
    if dev.type == "cuda":
        if args.vram_gb is None:
            raise SystemExit("--vram-gb is required when CUDA is used")
        total = torch.cuda.get_device_properties(0).total_memory / 2 ** 30
        torch.cuda.set_per_process_memory_fraction(min(1.0, args.vram_gb / total), 0)
        print(f"device cuda, {args.vram_gb:g} GB of {total:.1f} GB cap", flush=True)
    else:
        print("device cpu", flush=True)

    print(f"loading {args.ckpt}", flush=True)
    info = fixed_eval.load_ckpt(str(args.ckpt), dev)
    ppo.FORCE_DARK = info["force_dark_gift"]
    ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = info["old_boards"]
    ppo.REAL_LOBBY_FILTER = True
    print(f"  update {info['update']}, force_dark_gift {info['force_dark_gift']}, "
          f"old_boards {info['old_boards']}, comp inputs {info['n_comps']}", flush=True)

    ids = ref_ids()
    power_ids, trinkets = power_and_trinket_tables()
    started = time.time()
    pairs: list[dict] = []
    for lag in lags:
        lag_from = 1 if lag == 0 else args.lag2_from
        print(f"lag {lag} from turn {lag_from}", flush=True)
        pairs.extend(collect_lag(args, info, real, lag, lag_from, dev, ids,
                                 power_ids, trinkets))

    written = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    lags_text = ", ".join(
        f"lag {lag}" + ("" if lag == 0 else f" from turn {args.lag2_from}")
        for lag in lags)
    payload = {
        "checkpoint": str(args.ckpt),
        "update": info["update"],
        "written": written,
        "git": fixed_eval.git_rev(),
        "games": args.games,
        "seed_base": args.seed_base,
        "chunk": fixed_eval.CHUNK,
        "lineup": fixed_eval.LINEUP,
        "boards": str(folder),
        "n_board_files": n_board_files,
        "lags": lags,
        "lags_text": lags_text,
        "elapsed_seconds": round(time.time() - started, 1),
        "pairs": pairs,
    }
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "pairs.json").write_text(json.dumps(payload) + "\n")
    print(f"wrote {args.out_dir / 'pairs.json'} ({len(pairs)} pairs, "
          f"{payload['elapsed_seconds']}s)", flush=True)


# ---- fights ---------------------------------------------------------------


def plain(ids, template) -> dict:
    return {
        "card_id": card_id(ids, template["name"]),
        "name": template["name"],
        "attack": int(template["attack"]),
        "health": int(template["health"]),
        "golden": False,
        "keywords": list(template["keywords"]),
    }


def median_copy(ids, minions) -> dict:
    ordered = sorted(minions, key=lambda m: (m["attack"] + m["health"],
                                             m["name"]))
    # The lower median for an even count.
    m = ordered[(len(ordered) - 1) // 2]
    return {
        "card_id": m["card_id"],
        "name": m["name"],
        "attack": int(m["attack"]),
        "health": int(m["health"]),
        "golden": False,
        "keywords": list(m["keywords"]),
    }


def weakest(minions) -> int:
    return min(range(len(minions)),
               key=lambda i: (minions[i]["attack"] + minions[i]["health"], i))


def strongest(minions) -> int:
    return max(range(len(minions)),
               key=lambda i: (minions[i]["attack"] + minions[i]["health"], -i))


def with_slot(board, index, minion) -> dict:
    ms = [dict(m) for m in board["minions"]]
    ms[index] = dict(minion)
    return {**board, "minions": ms}


def without_slot(board, index) -> dict:
    ms = [dict(m) for m in board["minions"]]
    del ms[index]
    return {**board, "minions": ms}


def with_insert(board, pos, minion) -> dict:
    ms = [dict(m) for m in board["minions"]]
    ms.insert(pos, dict(minion))
    return {**board, "minions": ms}


def net_uncapped(result) -> float:
    return (result["damage_taken_uncapped_mean"]
            - result["damage_dealt_uncapped_mean"])


def one_fight(me, opp, seed) -> dict:
    bf = bgpy.BoardFight(json.dumps(me), json.dumps(opp), str(REF))
    r = bf.fight(FIGHTS, seed)
    return {
        "complete": bool(r["complete"]),
        "missing": list(r["missing"]),
        "win": r["win"],
        "tie": r["tie"],
        "loss": r["loss"],
        "damage_dealt_mean": r["damage_dealt_mean"],
        "damage_taken_mean": r["damage_taken_mean"],
        "damage_dealt_uncapped_mean": r["damage_dealt_uncapped_mean"],
        "damage_taken_uncapped_mean": r["damage_taken_uncapped_mean"],
    }


def best_insert(base, minion, opp, seed) -> tuple[dict, int, dict]:
    """Try the card in every insertion position of `base`; keep the lowest
    mean uncapped net damage (taken minus dealt), lowest position on a tie."""
    best = None
    best_pos = -1
    for pos in range(len(base["minions"]) + 1):
        r = one_fight(with_insert(base, pos, minion), opp, seed)
        if best is None or net_uncapped(r) < net_uncapped(best) - 1e-12:
            best, best_pos = r, pos
    return best, best_pos


def pair_seed(pair) -> int:
    return (pair["game"] * 1000 + pair["lag"] * 100 + pair["turn"]) & 0xFFFFFFFF


def baseline_worker(item):
    pair, ids = item
    try:
        r = one_fight(fight_board(pair["board"]),
                      fight_board(pair["opp_board"]), pair_seed(pair))
    except Exception as exc:
        return {"game": pair["game"], "lag": pair["lag"], "turn": pair["turn"],
                "error": f"{type(exc).__name__}: {exc}"}
    ratio = (sum(m["attack"] + m["health"] for m in pair["board"]["minions"])
             / max(1, sum(m["attack"] + m["health"]
                          for m in pair["opp_board"]["minions"])))
    return {
        "game": pair["game"],
        "lag": pair["lag"],
        "turn": pair["turn"],
        "ratio": ratio,
        "source": pair.get("opp_board_source", "unknown"),
        "actual": pair["actual"],
        "complete": r["complete"],
        "missing": r["missing"],
        "win": r["win"],
        "tie": r["tie"],
        "loss": r["loss"],
    }


def fight_board(board) -> dict:
    if KEEP_POWER_TRINKETS:
        return board
    return {k: v for k, v in board.items()
            if k not in ("hero_power", "trinkets")}


def arms_for(pair, ids) -> dict:
    board = fight_board(pair["board"])
    opp = fight_board(pair["opp_board"])
    seed = pair_seed(pair)
    minions = board["minions"]
    out = {}
    baseline = one_fight(board, opp, seed)
    out["Baseline"] = baseline
    if not minions:
        return out
    n = len(minions)
    wi = weakest(minions)
    si = strongest(minions)
    leeroy = plain(ids, LEEROY)
    spore = plain(ids, SPORE)
    control = median_copy(ids, minions)

    out["Leeroy weakest same slot"] = one_fight(
        with_slot(board, wi, leeroy), opp, seed)
    out["Spore weakest same slot"] = one_fight(
        with_slot(board, wi, spore), opp, seed)
    out["Control"] = one_fight(with_slot(board, wi, control), opp, seed)

    best, pos = best_insert(board, spore, opp, seed)
    out["Spore weakest best slot"] = best
    out["Spore weakest best slot"]["best_pos"] = pos

    base = without_slot(board, si)
    best, pos = best_insert(base, leeroy, opp, seed)
    out["Leeroy strongest best slot"] = best
    out["Leeroy strongest best slot"]["best_pos"] = pos
    best, pos = best_insert(base, spore, opp, seed)
    out["Spore strongest best slot"] = best
    out["Spore strongest best slot"]["best_pos"] = pos

    if n < bgpy.BOARD:
        out["Leeroy add"] = one_fight(with_insert(board, n, leeroy), opp, seed)
        out["Spore add"] = one_fight(with_insert(board, n, spore), opp, seed)
        out["Control add"] = one_fight(with_insert(board, n, control), opp, seed)
    return out


def worker(item):
    pair, ids = item
    try:
        arms = arms_for(pair, ids)
    except Exception as exc:  # a board the parser refuses: record it, keep going
        return {"game": pair["game"], "lag": pair["lag"], "turn": pair["turn"],
                "error": f"{type(exc).__name__}: {exc}"}
    ratio = (sum(m["attack"] + m["health"] for m in pair["board"]["minions"])
             / max(1, sum(m["attack"] + m["health"]
                          for m in pair["opp_board"]["minions"])))
    return {
        "game": pair["game"],
        "lag": pair["lag"],
        "turn": pair["turn"],
        "tier": pair["tier"],
        "opp_tier": pair["opp_tier"],
        "ratio": ratio,
        "actual": pair["actual"],
        "arms": arms,
    }


# ---- statistics -----------------------------------------------------------


def mean_ci(values):
    if not values:
        return None, None, None
    m = statistics.fmean(values)
    if len(values) < 2:
        return m, m, m
    sd = statistics.stdev(values)
    half = 1.96 * sd / math.sqrt(len(values))
    return m, m - half, m + half


def diff_ci(values, ref):
    ds = [a - r for a, r in zip(values, ref)]
    return mean_ci(ds)


def split_of(row, kind):
    if kind == "gap":
        for label, lo, hi in GAPS:
            if lo <= row["ratio"] < hi:
                return label
        return GAPS[-1][0]
    if kind == "turn":
        for label, lo, hi in TURNS:
            if lo <= row["turn"] <= hi:
                return label
        return TURNS[-1][0]
    if kind == "lag":
        return f"lag {row['lag']}"
    raise ValueError(kind)


def rows_for_arm(rows, arm):
    return [r for r in rows if r.get("arms") and arm in r["arms"]]


def arm_names(rows):
    order = ["Baseline", "Control", "Leeroy weakest same slot",
             "Spore weakest same slot", "Spore weakest best slot",
             "Leeroy strongest best slot", "Spore strongest best slot",
             "Leeroy add", "Spore add", "Control add"]
    seen = set()
    for r in rows:
        if r.get("arms"):
            seen.update(r["arms"])
    return [a for a in order if a in seen]


def fmt(x, digits=3):
    if x is None:
        return "n/a"
    return f"{x:.{digits}f}"


def fmt_pct(x):
    if x is None:
        return "n/a"
    return f"{100 * x:.1f}%"


def fmt_delta(x, lo, hi, unit=1.0, digits=3):
    if x is None:
        return "n/a"
    return f"{x * unit:+.{digits}f} [{lo * unit:+.{digits}f}, {hi * unit:+.{digits}f}]"


def metric(result, name):
    if name == "win":
        return result["win"]
    if name == "tie":
        return result["tie"]
    if name == "loss":
        return result["loss"]
    if name == "taken":
        return result["damage_taken_mean"]
    if name == "taken_unc":
        return result["damage_taken_uncapped_mean"]
    if name == "net":
        return result["damage_dealt_mean"] - result["damage_taken_mean"]
    if name == "net_unc":
        return (result["damage_dealt_uncapped_mean"]
                - result["damage_taken_uncapped_mean"])
    raise ValueError(name)


METRICS = ("win", "tie", "loss", "taken", "taken_unc", "net", "net_unc")


def arm_summary(rows, arm, ref_arm):
    vals = []
    refs = []
    for r in rows:
        if not r.get("arms") or arm not in r["arms"]:
            continue
        if ref_arm is not None and ref_arm not in r["arms"]:
            continue
        vals.append(r["arms"][arm])
        if ref_arm is not None:
            refs.append(r["arms"][ref_arm])
    if not vals:
        return None
    out = {"n": len(vals)}
    for key in METRICS:
        xs = [metric(v, key) for v in vals]
        out[key] = statistics.fmean(xs)
        out[key + "_ci"] = mean_ci(xs)[1:]
    if ref_arm is not None:
        for key in METRICS:
            xs = [metric(v, key) for v in vals]
            rs = [metric(v, key) for v in refs]
            d, lo, hi = diff_ci(xs, rs)
            out["d_" + key] = d
            out["d_" + key + "_ci"] = (lo, hi)
        up10 = sum(1 for v, r in zip(vals, refs)
                   if v["win"] - r["win"] >= 0.10)
        out["up10"] = up10 / len(vals)
    return out


def split_rows(rows, kind):
    if kind == "gap":
        specs = [(label, lambda r, lo=lo, hi=hi: lo <= r["ratio"] < hi)
                 for label, lo, hi in GAPS]
    elif kind == "turn":
        specs = [(label, lambda r, lo=lo, hi=hi: lo <= r["turn"] <= hi)
                 for label, lo, hi in TURNS]
    else:
        specs = [(f"lag {lag}", lambda r, lag=lag: r["lag"] == lag)
                 for lag in sorted({r["lag"] for r in rows})]
    groups = {label: [r for r in rows if match(r)] for label, match in specs}
    return [label for label, _ in specs], groups


def delta_table(rows, arms, ref_arm, title):
    lines = [f"### {title}", ""]
    lines.append("| arm | d win (pp) | d taken capped | d taken uncapped | "
                 "d net capped | d net uncapped | +10pp win share |")
    lines.append("|---|---|---|---|---|---|---|")
    for arm in arms:
        s = arm_summary(rows, arm, ref_arm)
        if s is None:
            continue
        lines.append(
            f"| {arm} | "
            f"{fmt_delta(s.get('d_win'), *s.get('d_win_ci', (None, None)), unit=100, digits=1)} | "
            f"{fmt_delta(s.get('d_taken'), *s.get('d_taken_ci', (None, None)))} | "
            f"{fmt_delta(s.get('d_taken_unc'), *s.get('d_taken_unc_ci', (None, None)))} | "
            f"{fmt_delta(s.get('d_net'), *s.get('d_net_ci', (None, None)))} | "
            f"{fmt_delta(s.get('d_net_unc'), *s.get('d_net_unc_ci', (None, None)))} | "
            f"{fmt_pct(s.get('up10'))} |")
    lines.append("")
    return lines


# ---- analysis -------------------------------------------------------------


def diag_table(rows, labels_and_match, title):
    lines = [f"### {title}", ""]
    lines.append("| split | pairs | actual win | baseline win | difference (points) |")
    lines.append("|---|---|---|---|---|")
    for label, match in labels_and_match:
        sel = [r for r in rows if match(r)]
        if not sel:
            continue
        aw = sum(1 for r in sel if r["actual"] == "win") / len(sel)
        pw = statistics.fmean([r["win"] for r in sel])
        lines.append(f"| {label} | {len(sel)} | {fmt_pct(aw)} | {fmt_pct(pw)} | "
                     f"{100 * (pw - aw):+.2f} |")
    lines.append("")
    return lines


def analyze(args):
    global KEEP_POWER_TRINKETS
    KEEP_POWER_TRINKETS = args.with_power_trinkets
    args.out_dir.mkdir(parents=True, exist_ok=True)
    payload = json.loads(args.pairs.read_text())
    pairs = payload["pairs"]
    ids = ref_ids()
    print(f"{len(pairs)} pairs from {args.pairs}", flush=True)

    jobs = max(1, args.jobs)
    ctx = mp.get_context("fork") if jobs > 1 else None

    def run_all(fn, items):
        items = list(items)
        out = []
        started = time.time()
        if jobs == 1:
            for i, item in enumerate(items):
                out.append(fn(item))
                if (i + 1) % 500 == 0:
                    print(f"  {i + 1}/{len(items)}, {time.time() - started:.0f}s",
                          flush=True)
        else:
            with ctx.Pool(jobs) as pool:
                for i, row in enumerate(pool.imap(fn, items, chunksize=8)):
                    out.append(row)
                    if (i + 1) % 500 == 0:
                        print(f"  {i + 1}/{len(items)}, {time.time() - started:.0f}s",
                              flush=True)
        print(f"  {len(out)} in {time.time() - started:.0f}s", flush=True)
        return out

    # Sanity first: one 100-fight baseline per pair, before any arm work.
    base_rows = run_all(baseline_worker, ((p, ids) for p in pairs))
    errors = [r for r in base_rows if "error" in r]
    sanity_rows = [r for r in base_rows if "error" not in r]
    incomplete = sum(1 for r in sanity_rows if not r["complete"])
    missing = {}
    for r in sanity_rows:
        for line in r["missing"]:
            missing[line] = missing.get(line, 0) + 1
    actual = {"win": 0, "tie": 0, "loss": 0}
    for r in sanity_rows:
        actual[r["actual"]] += 1
    n = len(sanity_rows)
    actual_share = {k: v / n for k, v in actual.items()} if n else {}
    predicted = {k: statistics.fmean([r[k] for r in sanity_rows])
                 for k in ("win", "tie", "loss")} if sanity_rows else {}
    diffs = {k: predicted[k] - actual_share[k] for k in ("win", "tie", "loss")}
    worst = max(abs(d) for d in diffs.values()) if diffs else 0.0
    sanity = {
        "pairs": n,
        "actual": actual,
        "actual_share": actual_share,
        "predicted": predicted,
        "diff_points": {k: 100 * d for k, d in diffs.items()},
        "worst_points": 100 * worst,
        "incomplete": incomplete,
        "errors": len(errors),
        "power_trinkets": KEEP_POWER_TRINKETS,
    }
    alt = None
    if not KEEP_POWER_TRINKETS:
        KEEP_POWER_TRINKETS = True
        alt_rows = run_all(baseline_worker, ((p, ids) for p in pairs))
        KEEP_POWER_TRINKETS = False
        alt_rows = [r for r in alt_rows if "error" not in r]
        if alt_rows:
            alt_pred = {k: statistics.fmean([r[k] for r in alt_rows])
                        for k in ("win", "tie", "loss")}
            alt_diff = {k: alt_pred[k] - actual_share[k]
                        for k in ("win", "tie", "loss")}
            alt = {
                "predicted": alt_pred,
                "diff_points": {k: 100 * d for k, d in alt_diff.items()},
                "worst_points": 100 * max(abs(d) for d in alt_diff.values()),
            }
            sanity["with_power_trinkets"] = alt
    print(f"sanity: actual {actual_share}, predicted {predicted}, "
          f"worst {100 * worst:.2f} points, incomplete {incomplete}, "
          f"errors {len(errors)}", flush=True)
    (args.out_dir / "sanity.json").write_text(json.dumps(sanity, indent=2) + "\n")
    if worst > 0.02 and not args.force:
        print("STOP: the baseline misses the actual shares by more than 2 points.",
              flush=True)
        print("The saved boards are missing something; run with --force only to "
              "see the rest, not to trust it.", flush=True)
        stop = ["# Swap check: stopped at the sanity check", ""]
        stop.append(f"- checkpoint: `{payload['checkpoint']}` (update {payload['update']})")
        stop.append(f"- pairs: {n}; {incomplete} baseline boards `complete()` rejects")
        stop.append("")
        stop.append("## Result")
        stop.append("")
        stop.append("The baseline's win, tie and loss shares miss the results the "
                    "games recorded by more than 2 points, so the saved boards are "
                    "missing something and the rest of the analysis was not run.")
        stop.append("")
        stop.append("| result | actual share | baseline share | difference (points) |")
        stop.append("|---|---|---|---|")
        for key in ("win", "tie", "loss"):
            stop.append(f"| {key} | {fmt_pct(actual_share[key])} | "
                        f"{fmt_pct(predicted[key])} | {100 * diffs[key]:+.2f} |")
        stop.append("")
        stop.append(f"- worst difference: {100 * worst:.2f} points (limit 2)")
        if alt is not None:
            stop.append(f"- with the observed hero power and trinkets passed in: "
                        f"{alt['worst_points']:.2f} points")
        stop.append(f"- fights the parser refused: {len(errors)}")
        if missing:
            top = sorted(missing.items(), key=lambda x: -x[1])[:12]
            stop.append("- most common missing lines: "
                        + "; ".join(f"{line} ({count})" for line, count in top))
        stop.append("")
        stop.append("## Not reported")
        stop.append("")
        stop.append("The four readings on boards under half the opponent's stats, "
                    "all seven swap arms, the control, the add variants and the "
                    "stat-gap crossover are implemented in "
                    "`train/evaltools/swap_check.py`, but they were not run for "
                    "the report: the brief says to stop when the baseline misses "
                    "the recorded results by more than 2 points, and the rest "
                    "would be wrong.")
        stop.append("")
        stop.append("## Where the miss comes from")
        stop.append("")
        stop.append("The miss is not spread evenly. `observation_last_seen` rows "
                    "are fights the learner survived, where the next observation "
                    "carried the opponent board that went into the fight. "
                    "`env.seats` rows are fights that ended the game: the learner "
                    "died and the env reset before a later observation could be "
                    "read, so those rows keep the opponent board from the "
                    "learner's End Turn. The learner was the last shop before "
                    "combat in those rows (otherwise the row would say "
                    "`latest_snapshot`), so the minions are the last ones seen; "
                    "the pair can still have changed at combat, and the pieces "
                    "below are still missing.")
        stop.append("")
        stop.extend(diag_table(
            sanity_rows,
            [(label, lambda r, label=label: r["source"] == label)
             for label in sorted({r["source"] for r in sanity_rows})],
            "By opponent board source"))
        stop.extend(diag_table(
            sanity_rows,
            [(label, lambda r, lo=lo, hi=hi: lo <= r["ratio"] < hi)
             for label, lo, hi in GAPS],
            "By stat gap"))
        stop.append("The stat-gap rows are the clearest sign: the simulation "
                    "is more decisive than the games. Boards at or above the "
                    "opponent's stats win far more often than they did, and "
                    "boards just under half win slightly less, so the saved "
                    "fights are missing the effects that made real outcomes "
                    "less dependent on raw stats.")
        stop.append("")
        stop.extend(diag_table(
            sanity_rows,
            [(f"lag {lag}", lambda r, lag=lag: r["lag"] == lag)
             for lag in sorted({r["lag"] for r in sanity_rows})],
            "By lag"))
        stop.extend(diag_table(
            sanity_rows,
            [(label, lambda r, lo=lo, hi=hi: lo <= r["turn"] <= hi)
             for label, lo, hi in TURNS],
            "By turn"))
        stop.append("## What the boards carry")
        stop.append("")
        stop.append("- Minions with stats, keywords, golden flags and position, "
                    "the turn and tier, and the Blood Gem and Tavern spell "
                    "counters for both sides.")
        stop.append("- The observation also exposes the learner's hero power "
                    "and both sides' trinkets. Passing them to the fight made "
                    "the sanity check worse, because the observation does not "
                    "carry the power's numbers, its pressed state, the power "
                    "Lock and Load still owes, or a trinket's current value. "
                    "This run leaves them out; `--with-power-trinkets` includes "
                    "them and the same check misses further.")
        stop.append("- The saved board does not carry Dark Gifts, minion "
                    "`numbers` (a scaling card's count), the opponent's other "
                    "player-level counters (beetle, undead, spells cast) or a "
                    "hand, because the observation does not expose them for "
                    "both sides.")
        stop.append("")
        (args.out_dir / "report.md").write_text("\n".join(stop) + "\n")
        (args.out_dir / "rows.json").write_text(json.dumps({
            "stage": "sanity",
            "sanity": sanity,
            "pairs": sanity_rows,
            "missing": missing,
            "arms": [],
        }) + "\n")
        print(f"wrote {args.out_dir / 'report.md'} and rows.json", flush=True)
        return 1

    # The baseline passed: now the arms.
    rows = run_all(worker, ((p, ids) for p in pairs))
    errors = [r for r in rows if "error" in r]
    good = [r for r in rows if "error" not in r and r.get("arms")]
    base_complete = [r for r in good
                     if r["arms"]["Baseline"]["complete"]]
    # Keep only pairs whose baseline built completely for the arm analysis.
    good = base_complete
    arms = arm_names(good)

    under_half = [r for r in good if r["ratio"] < 0.5]
    out_rows = []
    for r in good:
        row = dict(r)
        row["ratio"] = round(r["ratio"], 6)
        out_rows.append(row)

    def same_vs_best(same, best, key):
        vals = []
        refs = []
        for r in under_half:
            if not r.get("arms") or same not in r["arms"] or best not in r["arms"]:
                continue
            vals.append(metric(r["arms"][best], key))
            refs.append(metric(r["arms"][same], key))
        return diff_ci(vals, refs)

    lines = ["# Swap check: does a scam card pay on the bot's boards?", ""]
    lines.append(f"- checkpoint: `{payload['checkpoint']}` (update {payload['update']})")
    lines.append(f"- games per lag: {payload['games']}, seeds {payload['seed_base']} "
                 f"onwards, chunk {payload['chunk']}, lineup {payload['lineup']}")
    lines.append(f"- frozen boards: `{payload['boards']}` "
                 f"({payload['n_board_files']} files, manifest checked)")
    lines.append(f"- lags: {payload['lags_text']}")
    lines.append("- a board is the board at the learner's End Turn, the one that "
                 "fought; the opponent is the one the engine paired with it")
    lines.append("- ghost fights left out; fights whose actual result could not "
                 "be read are left out")
    lines.append(f"- pairs: {n} with a result; {incomplete} baseline boards "
                 "`complete()` rejects")
    lines.append(f"- fights per pair and arm: {FIGHTS}, same seed across arms")
    lines.append(f"- written: {payload.get('written', 'unknown')}")
    lines.append(f"- analyzed: {datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')}")
    lines.append("")
    lines.append("## Four readings Fable will use")
    lines.append("")
    lines.append("Boards under half the opponent's Attack plus Health "
                 f"(ratio < 0.5): {len(under_half)} pairs. Best-slot arms are "
                 "paired against the control; ranges are 95% CIs over pairs. "
                 "Damage deltas are positive when the arm takes or nets more.")
    lines.append("")
    lines.append("| arm | d uncapped taken | d capped taken | d win (pp) |")
    lines.append("|---|---|---|---|")
    for arm in ("Spore weakest best slot", "Leeroy strongest best slot",
                "Spore strongest best slot"):
        s = arm_summary(under_half, arm, "Control")
        if s is None:
            continue
        lines.append(
            f"| {arm} vs control | "
            f"{fmt_delta(s['d_taken_unc'], *s['d_taken_unc_ci'])} | "
            f"{fmt_delta(s['d_taken'], *s['d_taken_ci'])} | "
            f"{fmt_delta(s['d_win'], *s['d_win_ci'], unit=100, digits=1)} |")
    lines.append("")
    lines.append("Same-slot against best-slot on the same boards:")
    lines.append("")
    lines.append("| pair | d uncapped taken | d capped taken | d win (pp) |")
    lines.append("|---|---|---|---|")
    for same, best in (("Spore weakest same slot", "Spore weakest best slot"),):
        vals = []
        for key, unit in (("taken_unc", 1.0), ("taken", 1.0), ("win", 100.0)):
            vals.append(fmt_delta(*same_vs_best(same, best, key), unit=unit,
                                   digits=1 if unit == 100 else 3))
        lines.append(f"| {best} minus {same} | " + " | ".join(vals) + " |")
    lines.append("")

    lines.extend(["## Sanity check", ""])
    lines.append("The baseline fights 100 times for every pair; its win, tie and "
                 "loss shares are compared with the results the games recorded.")
    lines.append("")
    lines.append("| result | actual share | baseline share | difference (points) |")
    lines.append("|---|---|---|---|")
    for key in ("win", "tie", "loss"):
        lines.append(f"| {key} | {fmt_pct(actual_share[key])} | "
                     f"{fmt_pct(predicted[key])} | "
                     f"{100 * diffs[key]:+.1f} |")
    lines.append("")
    lines.append(f"- worst difference: {100 * worst:.2f} points (limit 2)")
    lines.append(f"- pairs `complete()` rejects: {incomplete} of {n}")
    if missing:
        top = sorted(missing.items(), key=lambda x: -x[1])[:12]
        lines.append("- most common missing lines: "
                     + "; ".join(f"{line} ({count})" for line, count in top))
    lines.append(f"- fights the parser refused: {len(errors)}")
    lines.append("")

    lines.extend(["## Overall arms", ""])
    lines.append("### Absolute shares and mean damage")
    lines.append("")
    lines.append("| arm | pairs | win | tie | loss | taken capped | taken uncapped "
                 "| net capped | net uncapped |")
    lines.append("|---|---|---|---|---|---|---|---|---|")
    for arm in arms:
        s = arm_summary(good, arm, None)
        if s is None:
            continue
        lines.append(
            f"| {arm} | {s['n']} | {fmt_pct(s['win'])} | {fmt_pct(s['tie'])} | "
            f"{fmt_pct(s['loss'])} | {fmt(s['taken'])} | {fmt(s['taken_unc'])} | "
            f"{fmt(s['net'])} | {fmt(s['net_unc'])} |")
    lines.append("")
    lines.extend(delta_table(good, arms, "Baseline", "Paired against the baseline"))
    lines.extend(delta_table(good, arms, "Control", "Paired against the control"))

    lines.extend(["## Splits", ""])
    for kind, title in (("gap", "Stat gap (learner total over opponent total)"),
                        ("turn", "Turn"), ("lag", "Lag")):
        labels, groups = split_rows(good, kind)
        lines.append(f"### By {title.lower()}")
        lines.append("")
        lines.append("| band | pairs | share of pairs |")
        lines.append("|---|---|---|")
        for label in labels:
            g = groups[label]
            lines.append(f"| {label} | {len(g)} | "
                         f"{fmt_pct(len(g) / len(good) if good else None)} |")
        lines.append("")
        for label in labels:
            g = groups[label]
            if not g:
                continue
            lines.extend(delta_table(g, arms, "Baseline",
                                     f"{title}: {label}, against baseline"))
            lines.extend(delta_table(g, arms, "Control",
                                     f"{title}: {label}, against control"))

    lines.extend(["### By whether the learner's tier allowed buying the card", ""])
    for card, tier in (("Leeroy", LEEROY_TIER), ("Spore", SPORE_TIER)):
        card_arms = [a for a in arms if a.startswith(card)]
        for allowed in (True, False):
            g = [r for r in good if (r["tier"] >= tier) == allowed]
            label = (f"{card}: learner tier {'>=' if allowed else '<'} {tier} "
                     f"({len(g)} pairs)")
            lines.extend(delta_table(g, card_arms, "Baseline",
                                     label + ", against baseline"))
            lines.extend(delta_table(g, card_arms, "Control",
                                     label + ", against control"))
    lines.append("")

    lines.extend(["## Where the strongest body is worth less than the scam card", ""])
    lines.append("For each stat-gap band, the two 'for the strongest' arms against "
                 "the baseline. A negative d net uncapped means the swap improved "
                 "the board's uncapped net damage (dealt minus taken); a positive "
                 "d taken uncapped means it took more.")
    lines.append("")
    lines.append("| band | pairs | share | Leeroy d net unc | Leeroy d taken unc "
                 "| Spore d net unc | Spore d taken unc |")
    lines.append("|---|---|---|---|---|---|---|")
    for label, _, _ in GAPS:
        g = [r for r in good if split_of(r, "gap") == label]
        if not g:
            continue
        cells = [label, str(len(g)), fmt_pct(len(g) / len(good))]
        for arm in ("Leeroy strongest best slot", "Spore strongest best slot"):
            s = arm_summary(g, arm, "Baseline")
            cells.append(fmt_delta(s["d_net_unc"], *s["d_net_unc_ci"]) if s else "n/a")
            cells.append(fmt_delta(s["d_taken_unc"], *s["d_taken_unc_ci"]) if s else "n/a")
        lines.append("| " + " | ".join(cells) + " |")
    lines.append("")

    lines.extend(["## Limits", ""])
    lines.append("- Plain copies only: no golden, no buffs the bot might have added.")
    lines.append("- The swap is free. In a game the card costs Gold and a Tavern "
                 "slot, so this is an upper bound on what scam is worth.")
    lines.append("- Era's side of the scam check rested on 85 boards, so compare "
                 "with it loosely.")
    lines.append("- The boards carry minion stats, keywords, golden flags, tier, "
                 "turn and the Blood Gem and Tavern spell counters for both sides. "
                 "The observation does not expose hero powers, trinkets, the other "
                 "player-level counters (beetle, undead, spells cast) or a hand for "
                 "the opponent, so those fights are plain boards. The sanity check "
                 "above is the measure of how much that costs.")
    lines.append("- `best slot` tries every insertion position of the board after "
                 "the replaced minion is taken out (7 on a full board). The add "
                 "variants append the card at the end.")
    lines.append("- The median minion for the control is the lower median on an "
                 "even board, copied at its current stats and with its keywords, "
                 "not golden.")
    lines.append("- Pairs are a turn-end board and the paired opponent; a shop-phase "
                 "knockout in the final action is not detected, as in the scam check.")
    lines.append("")

    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "rows.json").write_text(json.dumps({
        "sanity": sanity,
        "pairs": out_rows,
        "missing": missing,
        "arms": arms,
    }) + "\n")
    (args.out_dir / "report.md").write_text("\n".join(lines) + "\n")
    print(f"wrote {args.out_dir / 'report.md'} and rows.json", flush=True)
    return 0


def main():
    args = parse_args()
    if args.stage == "collect":
        collect(args)
    else:
        raise SystemExit(analyze(args))


if __name__ == "__main__":
    main()
