"""One fixed yardstick for every checkpoint.

    .venv/bin/python train/fixed_eval.py CKPT [CKPT2 ...] --vram-gb 10 \
        [--games 2048] [--name NAME] [--temp T]

One learner seat against seven real seats (lineup 1,0,0,0,0,7), one game per
seed, played to its end. Games run in chunks of CHUNK envs; chunk k is
`bgpy.VecEnv(n, SEED_BASE + k, ...)` and only each env's first game is kept.
Game index is chunk * CHUNK + env, so the same index is the same seed for
every checkpoint. The real boards are a frozen copy under boards/frozen/ and
must match its MANIFEST.txt.

The checkpoint's own force_dark_gift and old_boards are used, and a network
with a comp input gets the overlay probed for the comp index only (wheels
off: no forced move, no bonus). Actions are sampled, like ppo.py's eval,
with the network's logits divided by `--temp` (default 1.0, the old path).

The report (report.md) and the per-game rows (results.json) are written to
train/runs/fixed-eval/NAME/.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import subprocess
import sys
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

import numpy as np
import torch

import bgpy
import comp_specs
import ppo
import real_boards
from model import load as load_model
from model import masked
from ppo import F_COMP, LEARNER, Bufs, act, comp_onehot, force_mask

ROOT = Path(__file__).resolve().parent.parent
TRAIN = ROOT / "train"
FROZEN = ROOT / "boards" / "frozen" / "real-2026-11-10"
FROZEN_ACTIONS = ROOT / "boards" / "frozen" / "actions-2026-11-10"
OUT_ROOT = TRAIN / "runs" / "fixed-eval"

SEED_BASE = 20261110
CHUNK = 512
LINEUP = [1, 0, 0, 0, 0, 7]
TARGET_TURNS = (6, 8, 10, 12, 14, 16)
TARGET_TURNS_16 = (8, 10, 12, 14, 16)
TIER_TARGETS = (3, 4, 5, 6)
TRIBE_NAMES = ppo.TRIBE_NAMES
# The plan's tier-curve minimums (docs/notes/plan-2026-11-12.md, 2.1).
TIER_CURVE_MIN = {
    "level_turn2": 0.90,
    "tier_turn5": 2.85,
    "tier_turn8": 4.60,
}
# The choose_trinket actions' range: every trinket pick is TRINKET_BASE + k.
TRINKET_KIND = ppo.KINDS.index("choose_trinket")
TRINKET_BASE = sum(n for _, n in bgpy.ACTION_KINDS[:TRINKET_KIND])
TRINKET_CHOICES = bgpy.ACTION_KINDS[TRINKET_KIND][1]
# A measuring tool (train/evaltools/trinket_where.py) sets this to a list and
# gets one record per resolved learner trinket offer. None turns it off.
TRINKET_LOG: list[dict] | None = None
_STREAMER_COUNTS: Counter | None = None


def parse_args():
    a = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    a.add_argument("ckpts", nargs="+", help="checkpoint paths, first is the paired baseline")
    a.add_argument("--games", type=int, default=2048, help="games (default 2048)")
    a.add_argument("--temp", type=float, default=1.0,
                   help="divide the network's logits by this before sampling "
                        "(default 1.0; must be above 0)")
    a.add_argument("--trinket-pick", choices=["own", "popular", "random"], default="own",
                   help="force every learner trinket pick: own (the network, the default), "
                        "popular (the most streamer-picked offered trinket, ties by offer "
                        "slot), or random (uniform over the offered trinkets, seeded from "
                        "the game seed)")
    a.add_argument("--name", help="run folder under train/runs/fixed-eval")
    a.add_argument("--power-text", action="store_true",
                   help="build the hero-power and trinket text parts fresh with their gates at "
                        "0, even when the checkpoint has none: the gate-0 equivalence check")
    a.add_argument("--boards", type=Path, default=FROZEN,
                   help=f"frozen boards folder (default {FROZEN.relative_to(ROOT)})")
    a.add_argument("--seed-base", type=int, default=SEED_BASE,
                   help=f"first chunk's seed (default {SEED_BASE})")
    a.add_argument("--vram-gb", type=float,
                   help="cap this process's GPU memory; required when CUDA is used")
    a.add_argument("--cpu", action="store_true", help="run on the CPU")
    a.add_argument("--no-lobby-filter", action="store_true",
                   help="turn off the real seats' lobby filter (on by default)")
    a.add_argument("--real-lag", type=int, default=0,
                   help="lag the real seats' draw by this many turns (default 0); this is "
                        "also the lag of every Era-start game")
    a.add_argument("--real-lag-from", type=int, default=1,
                   help="first turn the real seats' lag applies on; before it they draw "
                        "their own turn as at lag 0 (default 1: every turn)")
    a.add_argument("--learner-heroes", default=None,
                   help="comma-separated hero names every learner seat is given instead of "
                        "the random hero deal (e.g. \"Trade Prince Gallywix\"); empty keeps "
                        "the normal deal")
    a.add_argument("--era-starts", nargs="?", const=str(FROZEN_ACTIONS), default=None,
                   help="start every game from Era's logged positions in this action folder "
                        f"(default {FROZEN_ACTIONS.relative_to(ROOT)} when Era starts are on)")
    a.add_argument("--era-turns", default="7,11",
                   help="MIN,MAX turn an Era start begins on (default 7,11)")
    a.add_argument("--era-split", choices=["holdout", "train", "all"], default=None,
                   help="which side of the source-game split to draw from (default holdout)")
    return a.parse_args()


def pcbusy_line() -> str:
    try:
        out = subprocess.run(["pcbusy"], capture_output=True, text=True, timeout=20)
    except Exception:
        return "unavailable"
    for line in out.stdout.splitlines():
        if line.strip():
            line = line.strip()
            return line if line.startswith("pcbusy:") else f"pcbusy: {line}"
    return "pcbusy: unavailable"


def git_rev() -> str:
    try:
        out = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"],
                             capture_output=True, text=True, check=True)
        return out.stdout.strip()
    except Exception:
        return "unknown"


def check_boards(folder: Path) -> int:
    """Refuse unless every file in `folder` matches MANIFEST.txt."""
    manifest = folder / "MANIFEST.txt"
    if not manifest.is_file():
        raise SystemExit(f"{folder}: no MANIFEST.txt, refusing to run")
    lines = [line.strip() for line in manifest.read_text().splitlines() if line.strip()]
    if not lines or not lines[0].lower().startswith("files:"):
        raise SystemExit(f"{manifest}: first line must be 'files: N'")
    try:
        declared = int(lines[0].split(":", 1)[1])
    except ValueError:
        raise SystemExit(f"{manifest}: bad file count {lines[0]!r}")
    entries = {}
    for line in lines[1:]:
        parts = line.split(None, 1)
        if len(parts) != 2:
            raise SystemExit(f"{manifest}: bad line {line!r}")
        digest, name = parts
        entries[name.strip()] = digest.lower()
    files = {p.name: p for p in folder.iterdir()
             if p.is_file() and p.name != "MANIFEST.txt"}
    missing = sorted(set(entries) - set(files))
    extra = sorted(set(files) - set(entries))
    if missing or extra or len(files) != declared:
        bits = []
        if missing:
            bits.append("missing " + ", ".join(missing[:5]))
        if extra:
            bits.append("extra " + ", ".join(extra[:5]))
        if len(files) != declared:
            bits.append(f"{len(files)} files, manifest says {declared}")
        raise SystemExit(f"{folder}: boards no longer match MANIFEST.txt: " + "; ".join(bits))
    for name, path in sorted(files.items()):
        got = hashlib.sha256(path.read_bytes()).hexdigest()
        if got != entries[name]:
            raise SystemExit(f"{folder}/{name}: sha256 {got}, manifest says {entries[name]}")
    return declared


def resolve_overlay(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute() and not path.exists():
        path = TRAIN / path
    return path


def load_ckpt(path: str, dev: torch.device, power_text: bool = False) -> dict:
    ck = torch.load(path, map_location="cpu", weights_only=False)
    cfg = ck.get("cfg") or {}
    net = load_model(path, dev, power_text=True if power_text else None).eval()
    overlay = cfg.get("overlay_comps")
    overlay_path = None
    if net.n_comps:
        if not overlay:
            raise SystemExit(f"{path}: network has {net.n_comps} comp inputs but no overlay_comps")
        overlay_path = resolve_overlay(overlay)
        if not overlay_path.is_file():
            raise SystemExit(f"{path}: overlay comps {overlay_path} does not exist")
        have = comp_specs.count(overlay_path)
        if have != net.n_comps:
            raise SystemExit(f"{path}: built for {net.n_comps} comps, "
                             f"{overlay_path} has {have}")
    return {
        "path": path,
        "update": ck.get("update"),
        "force_dark_gift": bool(cfg.get("force_dark_gift")),
        "old_boards": cfg.get("old_boards") or "keep",
        "overlay_comps": str(overlay_path) if overlay_path else None,
        "n_comps": net.n_comps,
        "power_text": power_text,
        "net": net,
    }


def board_tribe(tribes) -> str:
    tribes = tribes[:len(TRIBE_NAMES)]
    k = int(np.argmax(tribes))
    return TRIBE_NAMES[k] if tribes[k] >= 4 else "mixed"


def wilson(k: int, n: int, z: float = 1.96) -> tuple[float, float]:
    if n <= 0:
        return (float("nan"), float("nan"))
    p = k / n
    den = 1.0 + z * z / n
    centre = (p + z * z / (2 * n)) / den
    half = z * np.sqrt((p * (1 - p) + z * z / (4 * n)) / n) / den
    return (float(centre - half), float(centre + half))


def mean_range(values, z: float = 1.96) -> tuple[float, float, float]:
    """(mean, lower, upper) with a normal 95% range from the standard error."""
    v = np.asarray(values, dtype=np.float64)
    if len(v) == 0:
        return (float("nan"), float("nan"), float("nan"))
    mean = float(v.mean())
    if len(v) < 2:
        return (mean, mean, mean)
    se = float(v.std(ddof=1) / np.sqrt(len(v)))
    return (mean, mean - z * se, mean + z * se)


def sample_actions(net, o, mask, comp, temp: float):
    """Sample every network seat's action; temp 1.0 keeps ppo.act's path."""
    if temp == 1.0:
        acts, _, _, _ = act(net, o, mask, comp=comp)
        return acts
    with torch.no_grad():
        logits, _ = net(o, comp=comp)
    dist = torch.distributions.Categorical(logits=masked(logits, mask) / temp)
    return dist.sample()


def tier_curve(rows: list[dict]) -> dict:
    """The plan's three tier-curve numbers, as review2/tiercurve.py computes them."""
    t = np.array([r["tiers"] for r in rows], dtype=np.int64)
    ko = np.array([r["knocked_out_turn"] or 99 for r in rows], dtype=np.int64)

    def tier_at(turn: int) -> float | None:
        tt = np.zeros(len(rows), dtype=np.int64)
        for k in range(6):
            tt = np.where((t[:, k] > 0) & (t[:, k] <= turn), k + 1, tt)
        alive = ko > turn
        return float(tt[alive].mean()) if alive.any() else None

    return {
        "level_turn2": float((t[:, 1] == 2).mean()) if rows else None,
        "tier_turn5": tier_at(5),
        "tier_turn8": tier_at(8),
    }


def trinket_option(obs_row, slot: int) -> tuple[int, int]:
    """The (trinket table index, price) at one offer position of an observation."""
    at = bgpy.SCALAR_TRINKET_OPTION + slot * bgpy.TRINKET_OPTION_FEATURES
    return int(round(float(obs_row[at]))) - 1, int(round(float(obs_row[at + 1])))


def offered_trinket_slots(mask_row) -> list[int]:
    """The legal choose_trinket offer positions, in the offer's own order."""
    return [k for k in range(TRINKET_CHOICES) if mask_row[TRINKET_BASE + k]]


def streamer_pick_counts() -> Counter:
    """The streamers' clean trinket picks by name, as trinket_picks reads them."""
    global _STREAMER_COUNTS
    if _STREAMER_COUNTS is None:
        evaltools = str(TRAIN / "evaltools")
        if evaltools not in sys.path:
            sys.path.insert(0, evaltools)
        import trinket_picks
        _, counts, _ = trinket_picks.streamer_picks()
        _STREAMER_COUNTS = counts
    return _STREAMER_COUNTS


def popular_trinket_slot(ids: list[int], names: list[str],
                         counts: Counter) -> int:
    """The offer position of the most streamer-picked offered trinket.

    A trinket the streamers never pick counts 0. Ties go to the lowest offer
    position: the scan keeps the first maximum.
    """
    best, best_count = 0, -1
    for slot, i in enumerate(ids):
        c = counts.get(names[i], 0)
        if c > best_count:
            best, best_count = slot, c
    return best


def forced_trinket_slot(mode: str, ids: list[int], names: list[str] | None = None,
                        counts: Counter | None = None, rng=None) -> int | None:
    """The offer position a forced pick takes, or None for own.

    `ids` are the offered trinket table indices, in offer order.
    """
    if mode == "own" or not ids:
        return None
    if mode == "popular":
        return popular_trinket_slot(ids, names or [], counts or Counter())
    if mode != "random":
        raise ValueError(f"unknown trinket pick mode {mode!r}")
    if rng is None:
        raise ValueError("the random trinket pick needs a generator")
    return int(rng.randrange(len(ids)))


def apply_trinket_pick(acts, b, mode: str, names: list[str] | None,
                       counts: Counter | None, rngs: dict[int, random.Random],
                       game_seed_base: int) -> int:
    """Override every learner ChooseTrinket action in `acts`, in place.

    A non-trinket action is never touched, so an End Turn with an offer open
    still buys the engine's cheapest option: that is not the learner's pick.
    The random generator is one per game, seeded from the game's own seed.
    """
    if mode == "own":
        return 0
    changed = 0
    for e in range(len(acts)):
        a = int(acts[e])
        if not (TRINKET_BASE <= a < TRINKET_BASE + TRINKET_CHOICES):
            continue
        slots = offered_trinket_slots(b.mask[e])
        if not slots:
            continue
        ids = [trinket_option(b.obs[e], s)[0] for s in slots]
        rng = None
        if mode == "random":
            rng = rngs.get(e)
            if rng is None:
                rng = random.Random(game_seed_base + e)
                rngs[e] = rng
        pick = forced_trinket_slot(mode, ids, names, counts, rng)
        acts[e] = TRINKET_BASE + slots[pick]
        changed += 1
    return changed


def collect_trinket_offers(b, acts, start: int, turns, done) -> None:
    """Record each learner offer that this action resolves, when logging is on.

    `done` keeps the later games an env plays while the chunk finishes out of
    the log: only the first game of each index is the fixed eval's game.
    """
    if TRINKET_LOG is None:
        return
    for e in np.nonzero(b.obs[:, bgpy.SCALAR_PENDING_TRINKET] > 0)[0]:
        e = int(e)
        if done[e] or b.kind[e] != LEARNER:
            continue
        slots = offered_trinket_slots(b.mask[e])
        if not slots:
            continue
        opts = [trinket_option(b.obs[e], s) for s in slots]
        a = int(acts[e])
        if a == bgpy.END_TURN:
            # The engine's own pick when the turn ends with the offer open:
            # the cheapest option, the first one on a price tie.
            pick = min(range(len(opts)), key=lambda j: (opts[j][1], j))
            auto = True
        elif TRINKET_BASE <= a < TRINKET_BASE + TRINKET_CHOICES:
            pick = a - TRINKET_BASE
            auto = False
        else:
            # Freeze (or another legal pass): the offer stays open.
            continue
        TRINKET_LOG.append({
            "game": int(start + e),
            "turn": int(turns[e]),
            "offer_slot": int(b.obs[e, bgpy.SCALAR_TRINKET_OFFER_SLOT]),
            "offer": [i for i, _ in opts],
            "prices": [p for _, p in opts],
            "pick_slot": int(pick),
            "pick": int(opts[pick][0]),
            "auto": auto,
        })


def eval_checkpoint(info: dict, args, real, dev: torch.device) -> list[dict]:
    net = info["net"]
    ppo.FORCE_DARK = info["force_dark_gift"]
    ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = info["old_boards"]
    ppo.REAL_LOBBY_FILTER = not args.no_lobby_filter
    era_mode = args.era_folder is not None
    era_kw = {}
    if era_mode:
        era_kw = dict(era_starts=str(args.era_folder), era_share=1.0,
                      era_turn_min=args.era_turn_min, era_turn_max=args.era_turn_max,
                      era_split=args.era_split)
    names = None
    mode = getattr(args, "trinket_pick", "own")
    trinket_names = None
    streamer_counts = None
    rows: list[dict] = []
    chunk = 0
    while chunk * CHUNK < args.games:
        start = chunk * CHUNK
        n = min(CHUNK, args.games - start)
        env = bgpy.VecEnv(n, args.seed_base + chunk, LINEUP, real_boards=real,
                          old_boards=info["old_boards"],
                          real_lobby_filter=not args.no_lobby_filter,
                          real_lag=args.real_lag,
                          real_lag_from=args.real_lag_from,
                          learner_heroes=args.learner_heroes or None, **era_kw)
        if mode != "own" and trinket_names is None:
            trinket_names = env.trinket_names()
            if mode == "popular":
                streamer_counts = streamer_pick_counts()
        rngs: dict[int, random.Random] = {}
        if info["overlay_comps"]:
            env.set_overlay(info["overlay_comps"], 0.0)
        b = Bufs(n)
        env.kinds(b.kinds)
        done = np.zeros(n, dtype=bool)
        era_seats = np.asarray(env.era_seats(), dtype=np.int64)
        era_turns = np.asarray(env.era_turns(), dtype=np.int64)
        era_bodies = np.asarray(env.era_bodies(), dtype=np.int16)
        if era_mode and names is None:
            names = env.card_names()
        collect_turns = set(TARGET_TURNS)
        if era_mode:
            for t0 in era_turns:
                if t0:
                    collect_turns.update((int(t0), int(t0) + 2, int(t0) + 4, int(t0) + 6))
        board_totals = [{str(t): None for t in collect_turns} for _ in range(n)]
        last_names: list[list[str]] = [[] for _ in range(n)]
        rec: list[dict | None] = [None] * n
        ar = np.arange(n)
        torch.manual_seed(args.seed_base + chunk)
        steps = 0
        while not done.all():
            env.observe(b.obs, b.mask, b.seat, b.kind)
            turns = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
            # The board before the learner's last action of a target turn.
            # The last such action is normally EndTurn, so this is the board
            # the shop phase finished with, before combat.
            current = (~done) & (b.kind == LEARNER)
            wanted = current & np.isin(turns, sorted(collect_turns))
            for e in np.nonzero(current)[0]:
                e = int(e)
                s = int(b.seat[e])
                board = env.seats(e)[s][5]
                last_names[e] = [m[0] for m in board]
                if wanted[e]:
                    board_totals[e][str(int(turns[e]))] = int(
                        sum(m[2] + m[3] for m in board))
            o = torch.from_numpy(b.obs).to(dev)
            mask = force_mask(o, torch.from_numpy(b.mask).to(dev))
            comp_idx = np.full(n, -1, np.int64)
            comp = None
            if net.n_comps:
                env.overlay_probe(b.forced1, b.flags)
                comp_idx = b.flags[ar, b.seat, F_COMP] - 1
                comp = comp_onehot(comp_idx, net.n_comps, dev)
            acts = sample_actions(net, o, mask, comp, args.temp)
            acts_np = acts.cpu().numpy()
            apply_trinket_pick(acts_np, b, mode, trinket_names, streamer_counts,
                               rngs, (args.seed_base + chunk) << 32)
            collect_trinket_offers(b, acts_np, start, turns, done)
            kinds = b.kinds.copy()
            env.step(acts_np, b.places, b.tier_turns, b.tribes, b.reset)
            for e, s in zip(*np.nonzero(b.places)):
                e, s = int(e), int(s)
                if kinds[e, s] != LEARNER or done[e]:
                    continue
                place = int(b.places[e, s])
                row = {
                    "game": start + e,
                    "chunk": chunk,
                    "env": e,
                    "place": place,
                    "knocked_out_turn": None if place == 1 else int(turns[e]),
                    "tribe": board_tribe(b.tribes[e, s]),
                    "tiers": [int(x) for x in b.tier_turns[e, s]],
                    "board_totals": board_totals[e],
                }
                if era_mode:
                    e_seat = int(era_seats[e])
                    start_ids = [int(x) for x in era_bodies[e, e_seat] if x >= 0]
                    row["era_turn"] = int(era_turns[e])
                    row["era_body"] = [names[c] for c in start_ids]
                    row["final_names"] = list(last_names[e])
                rec[e] = row
                done[e] = True
            if b.reset.any():
                env.kinds(b.kinds)
            steps += 1
            if steps % 2000 == 0 and not done.all():
                print(f"    chunk {chunk}: {int(done.sum())}/{n} first games done",
                      flush=True)
        rows.extend(r for r in rec if r is not None)
        print(f"  chunk {chunk} done: {int(done.sum())}/{n} games, "
              f"{len(rows)}/{args.games} total, {steps} steps", flush=True)
        chunk += 1
    rows.sort(key=lambda r: r["game"])
    return rows


def summarize(rows: list[dict]) -> dict:
    places = np.array([r["place"] for r in rows], dtype=np.float64)
    n = len(places)
    mean, lo, hi = mean_range(places)
    top4 = int((places <= 4).sum())
    first = int((places == 1).sum())
    w4 = wilson(top4, n)
    w1 = wilson(first, n)
    ko = Counter(r["knocked_out_turn"] for r in rows
                 if r["place"] > 1 and r["knocked_out_turn"] is not None)
    tribes = {}
    for t in TRIBE_NAMES:
        sel = np.array([r["tribe"] == t for r in rows], dtype=bool)
        tribes[t] = {
            "count": int(sel.sum()),
            "place": float(places[sel].mean()) if sel.any() else None,
        }
    mixed = np.array([r["tribe"] == "mixed" for r in rows], dtype=bool)
    boards = {}
    for t in TARGET_TURNS:
        vals = [r["board_totals"][str(t)] for r in rows
                if r["board_totals"].get(str(t)) is not None]
        boards[t] = {
            "reached": len(vals),
            "mean": float(np.mean(vals)) if vals else None,
            "median": float(np.median(vals)) if vals else None,
        }
    # The same games for every turn, so growth is not read on survivors of
    # different lengths: only games with a board total at turn 16.
    at16 = [r for r in rows if r["board_totals"].get("16") is not None]
    board16 = {"games": len(at16), "turns": {}}
    for t in TARGET_TURNS_16:
        vals = [r["board_totals"][str(t)] for r in at16
                if r["board_totals"].get(str(t)) is not None]
        board16["turns"][t] = {
            "reached": len(vals),
            "mean": float(np.mean(vals)) if vals else None,
            "median": float(np.median(vals)) if vals else None,
        }
    def ratio(a, b):
        x = board16["turns"][a]["mean"]
        y = board16["turns"][b]["mean"]
        return x / y if x is not None and y else None
    board16["ratio_16_12"] = ratio(16, 12)
    board16["ratio_12_8"] = ratio(12, 8)
    tiers = {}
    for tier in TIER_TARGETS:
        vals = [r["tiers"][tier - 1] for r in rows if r["tiers"][tier - 1] > 0]
        tiers[tier] = {
            "reached": len(vals),
            "mean_turn": float(np.mean(vals)) if vals else None,
        }
    extra = {}
    if rows and "era_turn" in rows[0]:
        kept, total = 0, 0
        for r in rows:
            left = list(r["final_names"])
            for card in r["era_body"]:
                if card in left:
                    left.remove(card)
                    kept += 1
            total += len(r["era_body"])
        extra["era_body_kept"] = kept / total if total else float("nan")
        extra["era_body_total"] = total
        by_turn = {}
        for turn in sorted({r["era_turn"] for r in rows}):
            sel = [r for r in rows if r["era_turn"] == turn]
            vals = np.array([r["place"] for r in sel], dtype=np.float64)
            by_turn[turn] = {
                "games": len(sel),
                "place": float(vals.mean()) if len(vals) else None,
                "top4": float((vals <= 4).mean()) if len(vals) else None,
                "first": float((vals == 1).mean()) if len(vals) else None,
            }
        extra["era_place_by_turn"] = by_turn
        growth = {}
        for offset in (0, 2, 4, 6):
            vals = [r["board_totals"].get(str(r["era_turn"] + offset)) for r in rows]
            vals = [v for v in vals if v is not None]
            growth[offset] = {
                "reached": len(vals),
                "mean": float(np.mean(vals)) if vals else None,
                "median": float(np.median(vals)) if vals else None,
            }
        extra["era_boards"] = growth
    return {
        "n": n,
        "place": {"mean": mean, "lo": lo, "hi": hi},
        "top4": {"count": top4, "share": top4 / n if n else None,
                 "lo": w4[0], "hi": w4[1]},
        "first": {"count": first, "share": first / n if n else None,
                  "lo": w1[0], "hi": w1[1]},
        "places": {p: int((places == p).sum()) for p in range(1, 9)},
        "knocked_out": dict(sorted(ko.items())),
        "mixed": {"count": int(mixed.sum()),
                  "share": float(mixed.mean()) if n else None},
        "tribes": tribes,
        "boards": boards,
        "boards_at16": board16,
        "tiers": tiers,
        **extra,
    }


def pct(x: float | None) -> str:
    if x is None:
        return "n/a"
    if 0 < x < 0.001:
        return "<0.1%"
    return f"{100 * x:.1f}%"


def num(x: float | None, places: int = 3) -> str:
    return "n/a" if x is None else f"{x:.{places}f}"


def pass_fail(value: float | None, minimum: float) -> str:
    return "PASS" if value is not None and value >= minimum else "FAIL"


def build_report(name: str, args, n_board_files: int, rev: str,
                 infos: list[dict], results: list[dict]) -> str:
    lines = [f"# Fixed eval: {name}", ""]
    lines.append(f"- {pcbusy_line()}")
    lines.append(f"- boards: `{args.boards}` ({n_board_files} files, manifest checked)")
    lines.append(f"- seed base: {args.seed_base}")
    lines.append(f"- games: {args.games}, chunk size {CHUNK}, one game per seed")
    lines.append(f"- temp: {args.temp:g}")
    lines.append(f"- trinket pick: {args.trinket_pick}")
    lines.append(f"- lineup: {','.join(str(x) for x in LINEUP)} (learner then 7 real)")
    lines.append(f"- lobby filter: {'off' if args.no_lobby_filter else 'on'}")
    lines.append(f"- real lag: {args.real_lag} turn{'s' if args.real_lag != 1 else ''}")
    lines.append(f"- real lag from: turn {args.real_lag_from}")
    lines.append("- learner heroes: "
                 + (", ".join(args.learner_heroes) if args.learner_heroes
                    else "random (the normal deal)"))
    if args.power_text:
        lines.append("- power text: hero-power and trinket parts added fresh, gates at 0")
    if args.era_folder is not None:
        lines.append(f"- mode: Era starts ({args.era_split} split, turns "
                     f"{args.era_turn_min}..{args.era_turn_max}, "
                     f"`{args.era_folder}`)")
        lines.append(f"- Era start lag: {args.real_lag} (from --real-lag, the fixed "
                     "lag covers late starts too)")
    lines.append(f"- git: {rev}")
    lines.append(f"- bgpy: OBS_SIZE {bgpy.OBS_SIZE}, N_ACTIONS {bgpy.N_ACTIONS}")
    lines.append(f"- written: {datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')}")
    lines.append("")
    lines.append("| checkpoint | update | force_dark_gift | old_boards | comp inputs |")
    lines.append("|---|---|---|---|---|")
    for info in infos:
        lines.append(f"| `{info['path']}` | {info['update']} | "
                     f"{info['force_dark_gift']} | {info['old_boards']} | "
                     f"{info['n_comps']} |")
    lines.append("")

    for info, result in zip(infos, results):
        s = result["summary"]
        lines.append(f"## `{info['path']}` (update {info['update']})")
        lines.append("")
        lines.append(f"Average place: **{num(s['place']['mean'])}** "
                     f"(95% range {num(s['place']['lo'])} to {num(s['place']['hi'])}, "
                     f"{s['n']} games)")
        lines.append("")
        lines.append(f"Top 4: **{s['top4']['count']}** ({pct(s['top4']['share'])}, "
                     f"95% Wilson {pct(s['top4']['lo'])} to {pct(s['top4']['hi'])})")
        lines.append("")
        lines.append(f"Firsts: **{s['first']['count']}** ({pct(s['first']['share'])}, "
                     f"95% Wilson {pct(s['first']['lo'])} to {pct(s['first']['hi'])})")
        lines.append("")
        if "era_body_kept" in s:
            lines.append(f"Era body kept: **{pct(s['era_body_kept'])}** of the starting "
                         f"board's card copies ({s['era_body_total']} copies)")
            lines.append("")
            lines.append("### Place by Era starting turn")
            lines.append("")
            lines.append("| start turn | games | average place | top 4 | firsts |")
            lines.append("|---|---|---|---|---|")
            for turn, d in s["era_place_by_turn"].items():
                lines.append(f"| {turn} | {d['games']} | {num(d['place'], 2)} | "
                             f"{pct(d['top4'])} | {pct(d['first'])} |")
            lines.append("")
            lines.append("### Era seat's board total from its start turn")
            lines.append("")
            lines.append("| turns after start | games reached | mean total | median total |")
            lines.append("|---|---|---|---|")
            for offset, d in s["era_boards"].items():
                lines.append(f"| {offset} | {d['reached']} | {num(d['mean'], 1)} | "
                             f"{num(d['median'], 1)} |")
            lines.append("")
        lines.append("### Place distribution")
        lines.append("")
        lines.append("| place | games | share |")
        lines.append("|---|---|---|")
        for p in range(1, 9):
            count = s["places"][p]
            lines.append(f"| {p} | {count} | {pct(count / s['n'] if s['n'] else None)} |")
        lines.append("")
        lines.append("### Knocked out on turn")
        lines.append("")
        if s["knocked_out"]:
            lines.append("| turn | games |")
            lines.append("|---|---|")
            for turn, count in s["knocked_out"].items():
                lines.append(f"| {turn} | {count} |")
        else:
            lines.append("No games finished below first place.")
        lines.append("")
        lines.append("### Boards by tribe")
        lines.append("")
        lines.append(f"Mixed: **{s['mixed']['count']}** ({pct(s['mixed']['share'])})")
        lines.append("")
        lines.append("| tribe | games | share | average place |")
        lines.append("|---|---|---|---|")
        for t in TRIBE_NAMES:
            d = s["tribes"][t]
            lines.append(f"| {t} | {d['count']} | "
                         f"{pct(d['count'] / s['n'] if s['n'] else None)} | "
                         f"{num(d['place'], 2)} |")
        lines.append("")
        lines.append("### Board Attack plus Health by turn")
        lines.append("")
        lines.append("| turn | games reached | mean total | median total |")
        lines.append("|---|---|---|---|")
        for t in TARGET_TURNS:
            d = s["boards"][t]
            lines.append(f"| {t} | {d['reached']} | {num(d['mean'], 1)} | "
                         f"{num(d['median'], 1)} |")
        lines.append("")
        g = s["boards_at16"]
        lines.append(f"### Board Attack plus Health, games that reached turn 16 ({g['games']} games)")
        lines.append("")
        if g["games"]:
            lines.append("| turn | games | mean total | median total |")
            lines.append("|---|---|---|---|")
            for t in TARGET_TURNS_16:
                d = g["turns"][t]
                lines.append(f"| {t} | {d['reached']} | {num(d['mean'], 1)} | "
                             f"{num(d['median'], 1)} |")
            lines.append("")
            lines.append(f"Growth on these games: total at 16 divided by total at 12 = "
                         f"**{num(g['ratio_16_12'])}**; at 12 divided by 8 = "
                         f"**{num(g['ratio_12_8'])}**.")
        else:
            lines.append("No game reached turn 16.")
        lines.append("")
        lines.append("### Tier-up turns")
        lines.append("")
        lines.append("| tier | games reached | average turn |")
        lines.append("|---|---|---|")
        for tier in TIER_TARGETS:
            d = s["tiers"][tier]
            lines.append(f"| {tier} | {d['reached']} | {num(d['mean_turn'], 1)} |")
        lines.append("")
        tc = result["tier_curve"]
        lines.append("### Tier curve")
        lines.append("")
        lines.append(f"- Levelling on turn 2: **{pct(tc['level_turn2'])}** "
                     f"(minimum {100 * TIER_CURVE_MIN['level_turn2']:.0f}%, "
                     f"{pass_fail(tc['level_turn2'], TIER_CURVE_MIN['level_turn2'])})")
        lines.append(f"- Tier at the end of turn 5: **{num(tc['tier_turn5'], 2)}** "
                     f"(minimum {num(TIER_CURVE_MIN['tier_turn5'], 2)}, "
                     f"{pass_fail(tc['tier_turn5'], TIER_CURVE_MIN['tier_turn5'])})")
        lines.append(f"- Tier at the end of turn 8: **{num(tc['tier_turn8'], 2)}** "
                     f"(minimum {num(TIER_CURVE_MIN['tier_turn8'], 2)}, "
                     f"{pass_fail(tc['tier_turn8'], TIER_CURVE_MIN['tier_turn8'])})")
        lines.append("")

    if len(results) > 1:
        lines.extend(build_pairs(infos, results))
    return "\n".join(lines) + "\n"


def build_pairs(infos: list[dict], results: list[dict]) -> list[str]:
    base = results[0]["rows"]
    base_place = {r["game"]: r["place"] for r in base}
    base_top4 = {r["game"]: int(r["place"] <= 4) for r in base}
    base_first = {r["game"]: int(r["place"] == 1) for r in base}
    lines = [f"## Paired against `{infos[0]['path']}`", ""]
    lines.append("Differences are the later checkpoint minus the first, over the same "
                 "game indices. For place a negative difference is better; for top 4 "
                 "and firsts a positive difference (percentage points) is better.")
    lines.append("")
    lines.append("| checkpoint | place difference (95% range) | "
                 "top 4 difference, pp (95% range) | firsts difference, pp (95% range) |")
    lines.append("|---|---|---|---|")
    pairs = []
    for info, result in zip(infos[1:], results[1:]):
        rows = {r["game"]: r for r in result["rows"]}
        games = sorted(set(base_place) & set(rows))
        dp = [rows[g]["place"] - base_place[g] for g in games]
        d4 = [100 * (int(rows[g]["place"] <= 4) - base_top4[g]) for g in games]
        d1 = [100 * (int(rows[g]["place"] == 1) - base_first[g]) for g in games]
        mp, plo, phi = mean_range(dp)
        m4, l4, h4 = mean_range(d4)
        m1, l1, h1 = mean_range(d1)
        pairs.append((info, (mp, plo, phi), (m4, l4, h4), (m1, l1, h1), len(games)))
        lines.append(f"| `{info['path']}` | {num(mp)} ({num(plo)} to {num(phi)}) | "
                     f"{m4:+.1f} ({l4:+.1f} to {h4:+.1f}) | "
                     f"{m1:+.1f} ({l1:+.1f} to {h1:+.1f}) |")
    lines.append("")
    for info, place, top4, first, n in pairs:
        def word(rng):
            lo, hi = rng[1], rng[2]
            return "includes 0" if lo <= 0 <= hi else "does not include 0"
        lines.append(f"For `{info['path']}` against the first checkpoint ({n} paired games): "
                     f"the place range {word(place)}, the top 4 range {word(top4)}, "
                     f"and the firsts range {word(first)}.")
    lines.append("")
    return lines


def main():
    args = parse_args()
    if args.games < 1:
        raise SystemExit("--games must be at least 1")
    if not args.temp > 0:
        raise SystemExit(f"--temp {args.temp}: temperature must be above 0")
    args.learner_heroes = [name.strip() for name in (args.learner_heroes or "").split(",")
                           if name.strip()]
    ppo.LEARNER_HEROES = args.learner_heroes
    folder = args.boards.resolve()
    n_board_files = check_boards(folder)
    real, unknown = real_boards.load(folder)
    if not real:
        raise SystemExit(f"{folder}: no real boards")
    print(f"real boards from {folder}: {n_board_files} files, {len(real)} boards"
          + (f", {len(set(unknown))} unmatched minion names" if unknown else ""), flush=True)

    era_mode = args.era_starts is not None or args.era_split is not None
    n_action_files = 0
    if era_mode:
        try:
            args.era_turn_min, args.era_turn_max = (
                int(x) for x in args.era_turns.split(","))
        except ValueError:
            raise SystemExit(f"--era-turns {args.era_turns}: needs MIN,MAX")
        if args.era_turn_min < 1 or args.era_turn_max < args.era_turn_min:
            raise SystemExit(f"--era-turns {args.era_turns}: need 1 <= MIN <= MAX")
        args.era_split = args.era_split or "holdout"
        era_folder = Path(args.era_starts or FROZEN_ACTIONS).resolve()
        n_action_files = check_boards(era_folder)
        args.era_folder = era_folder
        print(f"Era starts from {era_folder}: {n_action_files} files, "
              f"{args.era_split} split, turns {args.era_turn_min}..{args.era_turn_max}, "
              f"real lag {args.real_lag} (covers late starts too)", flush=True)
    else:
        args.era_folder = None
        args.era_turn_min = args.era_turn_max = 0
        args.era_split = None

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

    name = args.name or Path(args.ckpts[0]).parent.name or Path(args.ckpts[0]).stem
    out = OUT_ROOT / name
    out.mkdir(parents=True, exist_ok=True)

    infos, results = [], []
    for path in args.ckpts:
        print(f"loading {path}", flush=True)
        info = load_ckpt(path, dev, args.power_text)
        print(f"  update {info['update']}, force_dark_gift {info['force_dark_gift']}, "
              f"old_boards {info['old_boards']}, comp inputs {info['n_comps']}", flush=True)
        rows = eval_checkpoint(info, args, real, dev)
        if len(rows) != args.games:
            raise SystemExit(f"{path}: got {len(rows)} games, wanted {args.games}")
        results.append({"rows": rows, "summary": summarize(rows),
                        "tier_curve": tier_curve(rows)})
        infos.append(info)
        del info["net"]
        if dev.type == "cuda":
            torch.cuda.empty_cache()

    report = build_report(name, args, n_board_files, git_rev(), infos, results)
    (out / "report.md").write_text(report)
    payload = {
        "name": name,
        "written": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "seed_base": args.seed_base,
        "games": args.games,
        "chunk_size": CHUNK,
        "temp": args.temp,
        "trinket_pick": args.trinket_pick,
        "lineup": LINEUP,
        "boards": {"path": str(args.boards), "files": n_board_files},
        "lobby_filter": not args.no_lobby_filter,
        "real_lag": args.real_lag,
        "real_lag_from": args.real_lag_from,
        "learner_heroes": args.learner_heroes,
        "era": {
            "path": str(args.era_folder) if args.era_folder is not None else None,
            "files": (n_action_files if args.era_folder is not None else 0),
            "split": args.era_split,
            "turns": [args.era_turn_min, args.era_turn_max],
        },
        "git": git_rev(),
        "obs_size": bgpy.OBS_SIZE,
        "n_actions": bgpy.N_ACTIONS,
        "tier_index": "index 0 is tier 1, index 5 is tier 6; 0 means never reached",
        "checkpoints": [
            {
                "path": info["path"],
                "update": info["update"],
                "force_dark_gift": info["force_dark_gift"],
                "old_boards": info["old_boards"],
                "overlay_comps": info["overlay_comps"],
                "n_comps": info["n_comps"],
                "summary": result["summary"],
                "tier_curve": result["tier_curve"],
                "rows": result["rows"],
            }
            for info, result in zip(infos, results)
        ],
    }
    (out / "results.json").write_text(json.dumps(payload, indent=2) + "\n")
    print()
    print(report)
    print(f"written to {out / 'report.md'} and {out / 'results.json'}")


if __name__ == "__main__":
    main()
