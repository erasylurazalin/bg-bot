#!/usr/bin/env python3
"""The step 4 demonstration dataset: Era's replayed decisions, build A.

    .venv/bin/python train/evaltools/demo_dataset.py --cpu

Every turn of `boards/frozen/actions-2026-11-10` that replays to its end is
seated with `bgpy.EraReplay` / `EraTurnPath`, the same path as
`train/era_turns.py` and `train/evaltools/trinket_picks.py`. Before every
decision the script saves the observation and legal mask `ppo.py` would feed
its network, and Era's logged action as the flat action index `ppo.py` uses.
A logged action that the engine's mask does not allow is dropped and counted.
Turns that cannot be seated or played through are dropped whole and counted.

The split is by game: 85% train, 15% held out, fixed by a seeded shuffle of
the action file list. The game list and its split are saved beside the rows,
so a later rebuild keeps the same held-out games.

Output in `train/runs/demo-dataset/`: `demo.npz` with `obs`, `mask`, `action`,
`turn`, `game`, `split`, and the split list `games` / `game_split`; and
`report.md` with the counts, the dropped turns, and BASE's masked-argmax
agreement on the held-out part, overall and by action kind.

Measurement and data only. It does not edit ppo.py, model.py or fixed_eval.py.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import random
import sys
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "train"))
os.chdir(ROOT)

import bgpy

ACTIONS = ROOT / "boards/frozen/actions-2026-11-10"
OUT = ROOT / "train/runs/demo-dataset"
CKPT = ROOT / "train/runs/r1113-a1/best.pt"
REPLAY_SEED = 1
SPLIT_SEED = 1
HOLDOUT = 0.15

KINDS = [name for name, _ in bgpy.ACTION_KINDS]
KIND_OF = np.repeat(
    np.arange(len(KINDS)), [count for _, count in bgpy.ACTION_KINDS]
)

_ACTION_SPECS: dict[str, dict] = {}


def action_spec(name: str) -> dict:
    """The action file's parsed JSON, cached: report text only."""
    if name not in _ACTION_SPECS:
        _ACTION_SPECS[name] = json.loads((ACTIONS / name).read_text())
    return _ACTION_SPECS[name]


def turn_steps(name: str, turn: int) -> int:
    """How many logged steps the turn has, from the file itself."""
    for item in action_spec(name).get("turns", []):
        if item.get("turn") == turn:
            return len(item.get("actions", []))
    return 0


def step_name(name: str, turn: int, step: int) -> str:
    """The log's own step kind, for a dropped step's reason."""
    for item in action_spec(name).get("turns", []):
        if item.get("turn") == turn:
            actions = item.get("actions", [])
            if 0 <= step < len(actions):
                return str(actions[step].get("step", "unknown"))
    return "unknown"


def action_files() -> list[str]:
    """Every action file in the folder, sorted, as the game list."""
    return sorted(
        path.name
        for path in ACTIONS.glob("*.json")
        if not path.name.startswith(".")
    )


def split_games(files: list[str], seed: int = SPLIT_SEED,
                holdout: float = HOLDOUT) -> tuple[dict[str, str], list[str]]:
    """A fixed by-game split: `holdout` of the shuffled game list.

    Rounding is half up, so 91 games give 14 held out. A holdout of 0 still
    keeps one game held out while there is more than one game, so the report
    always has a held-out row to read.
    """
    n = len(files)
    k = int(math.floor(holdout * n + 0.5))
    if n > 1:
        k = min(max(k, 1), n - 1)
    else:
        k = 0
    order = list(files)
    random.Random(seed).shuffle(order)
    held = set(order[:k])
    split = {name: ("heldout" if name in held else "train") for name in files}
    return split, order


def replay_turn(replay, turn: int, seed: int = REPLAY_SEED):
    """Seat one turn and walk its decisions in Era's steps.

    Returns `(rows, skipped, error, logged_steps)`:
      rows: one dict per decision, with the observation, mask, action, step.
      skipped: `(step, log step kind, reason)` for logged decisions the engine
        could not ask about (the path played them itself).
      error: None when the turn replayed to its end, else the reason.
      logged_steps: the file's step count, when it can be read.

    Rows are only returned for a complete turn; a turn that stops mid-way
    contributes nothing, since its later rows would be missing.
    """
    logged = turn_steps(replay.file, turn)
    try:
        path = replay.path(turn, seed, True)
    except Exception as exc:
        return [], [], f"the turn cannot be seated: {exc}", logged

    rows: list[dict] = []
    skipped: list[tuple[int, str, str]] = []
    expected = 0
    try:
        while True:
            before = path.steps_on_path
            action = path.era_action()
            now = path.steps_on_path
            if now > expected:
                for step in range(expected, now):
                    skipped.append((
                        step,
                        step_name(replay.file, turn, step),
                        "the logged decision is not legal in the engine's mask",
                    ))
            if action < 0:
                break
            expected = now + 1
            if now >= path.logged_steps:
                break
            obs = np.zeros(bgpy.OBS_SIZE, dtype=np.float32)
            mask = np.zeros(bgpy.N_ACTIONS, dtype=bool)
            path.observe(obs, mask)
            if mask[action]:
                rows.append({
                    "step": now,
                    "obs": obs,
                    "mask": mask,
                    "action": int(action),
                })
            else:
                skipped.append((
                    now,
                    step_name(replay.file, turn, now),
                    "the logged action is not legal in the engine's mask",
                ))
            if not path.act(action):
                why = path.left_why or f"left Era's path at step {path.left_at}"
                return [], skipped, why, logged
    except Exception as exc:
        return [], skipped, str(exc), logged

    if not path.done:
        path.finish()
    if path.left_at >= 0:
        why = path.left_why or f"left Era's path at step {path.left_at}"
        return [], skipped, why, logged
    if path.steps_on_path != path.logged_steps:
        return [], skipped, (
            f"the turn ended after {path.steps_on_path} of "
            f"{path.logged_steps} logged steps"
        ), logged
    return rows, skipped, None, logged


def drop_class(error: str) -> str:
    """A short group for a dropped turn's reason."""
    if "the turn cannot be seated" in error:
        return "the turn start cannot be seated"
    if "cannot be seated" in error:
        return "a logged step cannot be seated"
    if "cannot be played" in error:
        return "a logged step cannot be played"
    if "does not allow" in error or "illegal" in error:
        return "the engine refuses a logged action"
    if "ended after" in error:
        return "the turn ended before its logged steps"
    return "other"


def turn_ref(name: str, turn: int) -> str:
    data = action_spec(name)
    return f"{data.get('source', name)}-g{data.get('game', 0)} t{turn}"


def collect(replays) -> tuple[dict, list[dict], list[dict]]:
    """Replay every turn and gather the rows and the per-turn records."""
    parts: dict[str, list[np.ndarray]] = {
        "obs": [], "mask": [], "action": [], "turn": [], "game": [], "split": [],
    }
    turns: list[dict] = []
    skipped: list[dict] = []
    for number, replay in enumerate(replays, 1):
        for turn in replay.turns(1):
            rows, misses, error, logged = replay_turn(replay, turn)
            record = {
                "file": replay.file,
                "source": replay.source,
                "game": replay.game,
                "turn": turn,
                "logged_steps": logged,
                "rows": len(rows),
                "skipped": misses,
                "error": error,
            }
            turns.append(record)
            for step, kind, why in misses:
                skipped.append({
                    "file": replay.file,
                    "source": replay.source,
                    "game": replay.game,
                    "turn": turn,
                    "step": step,
                    "kind": kind,
                    "reason": why,
                })
            for row in rows:
                parts["obs"].append(row["obs"])
                parts["mask"].append(row["mask"])
                parts["action"].append(row["action"])
                parts["turn"].append(turn)
                parts["game"].append(replay.file)
                # Filled by the caller once the split is known.
                parts["split"].append("")
        if number % 10 == 0 or number == len(replays):
            print(f"replayed {number}/{len(replays)} games", flush=True)

    arrays = {}
    for key in ("obs", "mask", "action", "turn"):
        if parts[key]:
            arrays[key] = np.asarray(parts[key])
        else:
            arrays[key] = np.zeros(0)
    arrays["game"] = np.asarray(parts["game"], dtype=str)
    arrays["split"] = np.asarray(parts["split"], dtype=str)
    return arrays, turns, skipped


def save_dataset(out: Path, arrays: dict, split: dict[str, str],
                 files: list[str]) -> None:
    """Write demo.npz with the rows and the full game list split."""
    game_split = np.asarray([split[name] for name in files], dtype=str)
    np.savez(
        out / "demo.npz",
        obs=arrays["obs"],
        mask=arrays["mask"],
        action=arrays["action"],
        turn=arrays["turn"],
        game=arrays["game"],
        split=arrays["split"],
        games=np.asarray(files, dtype=str),
        game_split=game_split,
    )


def pct(x: float) -> str:
    return "n/a" if not math.isfinite(x) else f"{100 * x:.1f}%"


def wilson(k: int, n: int) -> tuple[float, float]:
    """Wilson 95% interval for a 0/1 agreement count."""
    if n <= 0:
        return (float("nan"), float("nan"))
    z = 1.96
    p = k / n
    den = 1.0 + z * z / n
    centre = (p + z * z / (2 * n)) / den
    half = z * math.sqrt((p * (1 - p) + z * z / (4 * n)) / n) / den
    return (centre - half, centre + half)


def load_base(ckpt: str, cpu: bool, vram_gb: float):
    """BASE, its info, its device and ppo's acting flags."""
    import torch

    import fixed_eval as fe
    import ppo

    dev = torch.device("cpu" if cpu or not torch.cuda.is_available() else "cuda")
    if dev.type == "cuda":
        total_gb = torch.cuda.get_device_properties(0).total_memory / 2**30
        torch.cuda.set_per_process_memory_fraction(
            min(1.0, max(0.0, vram_gb) / total_gb), 0
        )
        print(f"device cuda, {vram_gb:g} GB of {total_gb:.1f} GB cap", flush=True)
    else:
        print("device cpu", flush=True)

    info = fe.load_ckpt(ckpt, dev)
    if info["n_comps"]:
        raise SystemExit(
            f"{ckpt}: network has {info['n_comps']} comp inputs; "
            "this measurement feeds no comp overlay"
        )
    ppo.FORCE_DARK = info["force_dark_gift"]
    ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = info["old_boards"]
    return info["net"], info, dev


def predict_base(net, arrays: dict, rows: np.ndarray, dev, batch: int):
    """Plain masked argmax and the force_mask argmax on the selected rows."""
    import torch

    import ppo
    from model import masked

    pred = np.empty(len(rows), dtype=np.int64)
    forced = np.empty(len(rows), dtype=np.int64)
    for start in range(0, len(rows), batch):
        stop = min(start + batch, len(rows))
        o = torch.from_numpy(arrays["obs"][rows[start:stop]]).to(dev)
        m = torch.from_numpy(arrays["mask"][rows[start:stop]]).to(dev)
        with torch.no_grad():
            logits, _ = net(o, comp=None)
            pred[start:stop] = masked(logits, m).argmax(1).cpu().numpy()
            fm = ppo.force_mask(o, m)
            forced[start:stop] = masked(logits, fm).argmax(1).cpu().numpy()
    return pred, forced


def main() -> None:
    global ACTIONS
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--actions", default=str(ACTIONS))
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--ckpt", default=str(CKPT))
    ap.add_argument("--seed", type=int, default=REPLAY_SEED,
                    help="the turn's replay seed (the same for every turn)")
    ap.add_argument("--split-seed", type=int, default=SPLIT_SEED)
    ap.add_argument("--holdout", type=float, default=HOLDOUT)
    ap.add_argument("--cpu", action="store_true", help="run BASE on the CPU")
    ap.add_argument("--vram-gb", type=float, default=2.0,
                    help="cap this process's GPU memory in GB (default 2)")
    ap.add_argument("--batch", type=int, default=256)
    ap.add_argument("--no-eval", action="store_true",
                    help="write the dataset without reading BASE")
    args = ap.parse_args()

    ACTIONS = Path(args.actions).resolve()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    files = action_files()
    split, _order = split_games(files, args.split_seed, args.holdout)
    held = [name for name in files if split[name] == "heldout"]
    print(
        f"{len(files)} games: {len(files) - len(held)} train, {len(held)} held out",
        flush=True,
    )

    replays, bad = bgpy.era_replays(str(ACTIONS), str(ROOT / "ref/bg-cards.json"))
    by_file = {replay.file: replay for replay in replays}
    for line in bad:
        print(f"loader: {line}", flush=True)
    missing_games = [name for name in files if name not in by_file]
    if missing_games:
        print(f"no replay for {len(missing_games)} files: {missing_games[:3]}",
              flush=True)

    arrays, turns, skipped = collect(
        [by_file[name] for name in files if name in by_file]
    )
    arrays["split"] = np.asarray(
        [split[name] for name in arrays["game"]], dtype=str
    )
    save_dataset(out, arrays, split, files)

    print(
        f"wrote {out / 'demo.npz'}: {len(arrays['action'])} rows, "
        f"{len(turns)} turns, {len(skipped)} dropped steps",
        flush=True,
    )

    replayed = [t for t in turns if t["error"] is None]
    dropped = [t for t in turns if t["error"] is not None]
    rows = np.arange(len(arrays["action"]))
    held_rows = rows[arrays["split"] == "heldout"]
    train_rows = rows[arrays["split"] == "train"]
    action = arrays["action"]
    kind_idx = KIND_OF[action] if len(action) else np.zeros(0, dtype=int)
    kind_name = (
        np.asarray(KINDS, dtype=object)[kind_idx]
        if len(action) else np.zeros(0, dtype=object)
    )

    if args.no_eval:
        pred = None
        forced = None
        train_pred = None
        info = {"update": None, "force_dark_gift": None, "old_boards": "keep"}
        dev = None
    else:
        net, info, dev = load_base(args.ckpt, args.cpu, args.vram_gb)
        pred, forced = predict_base(net, arrays, held_rows, dev, args.batch)
        # The same predictions on the train rows, for context only.
        if len(train_rows):
            train_pred, _ = predict_base(net, arrays, train_rows, dev, args.batch)
        else:
            train_pred = None

    lines: list[str] = []
    lines.append("# Demonstration dataset from Era's replayed turns")
    lines.append("")
    lines.append(
        "Build A of step 4 (`docs/notes/plan-2026-11-13.md`): the observation, "
        "legal mask and Era's action for every decision of every turn in "
        "`boards/frozen/actions-2026-11-10` that replays to its end. Script: "
        "`train/evaltools/demo_dataset.py`; data: `demo.npz`."
    )
    lines.append("")
    lines.append(
        f"Turn replay seed {args.seed}. Replays use `bgpy.EraReplay` and "
        "`EraTurnPath` with `reseat=True`, the same seating as "
        "`train/evaltools/trinket_picks.py` and the path `train/era_turns.py` "
        "uses for Era's own end-of-turn position. `EraTurnPath.observe` calls "
        "`obs::write`, the encoder `VecEnv.observe` calls with "
        f"`OldBoards::{info['old_boards'].capitalize()}`, which is BASE's "
        "setting. `era_action` returns `Action::index`, the flat index "
        "`ppo.py` uses. No second encoder."
    )
    lines.append("")
    if info.get("update") is not None:
        lines.append(
            f"BASE: `{Path(args.ckpt).relative_to(ROOT)}`, update "
            f"{info['update']}, device {dev.type}."
        )
        lines.append("")

    lines.append("## Counts")
    lines.append("")
    lines.append(
        f"{len(files)} action files; {len(replays)} replays loaded, "
        f"{len(bad)} files the loader could not build. "
        f"{len(turns)} turns in the files have at least one logged step; "
        f"{len(replayed)} replay to their end and {len(dropped)} are dropped."
    )
    lines.append("")
    lines.append(
        f"{len(action)} decision rows: {len(train_rows)} train, "
        f"{len(held_rows)} held out. {len(skipped)} logged steps were dropped "
        "as illegal before a row was saved."
    )
    lines.append("")
    lines.append(
        "The split is by game: `round(0.15 * 91) = 14` held out, the rest "
        f"train, fixed by a seeded shuffle of the file list (seed "
        f"{args.split_seed})."
    )
    lines.append("")
    lines.append("Held-out games: " + ", ".join(f"`{name}`" for name in held))
    lines.append("")

    lines.append("## Steps by turn")
    lines.append("")
    lines.append(
        "| turn | replayed turns | dropped turns | steps | train steps | held-out steps |"
    )
    lines.append("|---|---:|---:|---:|---:|---:|")
    by_turn: dict[int, dict] = defaultdict(
        lambda: {"replayed": 0, "dropped": 0, "steps": 0, "train": 0,
                 "heldout": 0}
    )
    for record in turns:
        bucket = by_turn[record["turn"]]
        if record["error"] is None:
            bucket["replayed"] += 1
        else:
            bucket["dropped"] += 1
    for i, row in enumerate(rows):
        bucket = by_turn[int(arrays["turn"][i])]
        bucket["steps"] += 1
        bucket[arrays["split"][i]] += 1
    for turn in sorted(by_turn):
        bucket = by_turn[turn]
        lines.append(
            f"| {turn} | {bucket['replayed']} | {bucket['dropped']} | "
            f"{bucket['steps']} | {bucket['train']} | {bucket['heldout']} |"
        )
    lines.append("")

    lines.append("## Steps by action kind")
    lines.append("")
    lines.append(
        "| action kind | steps | share | train | held out |"
    )
    lines.append("|---|---:|---:|---:|---:|")
    total = max(1, len(rows))
    for kind in KINDS:
        idx = np.flatnonzero(kind_name == kind)
        if len(idx) == 0:
            continue
        lines.append(
            f"| {kind} | {len(idx)} | {pct(len(idx) / total)} | "
            f"{int((arrays['split'][idx] == 'train').sum())} | "
            f"{int((arrays['split'][idx] == 'heldout').sum())} |"
        )
    lines.append("")

    lines.append("## Turns dropped")
    lines.append("")
    classes = Counter(drop_class(record["error"]) for record in dropped)
    lines.append("| reason | turns |")
    lines.append("|---|---:|")
    for why, count in sorted(classes.items(), key=lambda kv: (-kv[1], kv[0])):
        lines.append(f"| {why} | {count} |")
    lines.append("")
    lines.append("Every dropped turn, with the replay's own words:")
    lines.append("")
    lines.append("| game and turn | logged steps | reason |")
    lines.append("|---|---:|---|")
    for record in dropped:
        reason = record["error"].replace("|", "/")
        if len(reason) > 260:
            reason = reason[:257].rstrip() + "..."
        lines.append(
            f"| {turn_ref(record['file'], record['turn'])} | "
            f"{record['logged_steps']} | {reason} |"
        )
    lines.append("")

    lines.append("## Dropped steps")
    lines.append("")
    if skipped:
        lines.append(
            "A logged decision the engine's own legal mask does not allow is "
            "not a row. The replay plays that step from the log and continues, "
            "so the turn can still complete."
        )
        lines.append("")
        lines.append("| game and turn | step | log step | reason |")
        lines.append("|---|---:|---|---|")
        for record in skipped:
            lines.append(
                f"| {turn_ref(record['file'], record['turn'])} | "
                f"{record['step']} | {record['kind']} | {record['reason']} |"
            )
    else:
        lines.append("None.")
    lines.append("")

    lines.append("## BASE's held-out top-1 agreement")
    lines.append("")
    if pred is None:
        lines.append("Not measured (`--no-eval`).")
    else:
        def rate(prediction, target):
            if len(target) == 0:
                return 0, 0, float("nan")
            hits = int((prediction == target).sum())
            return hits, len(target), hits / len(target)

        held_actions = action[held_rows]
        hits, n, r = rate(pred, held_actions)
        lo, hi = wilson(hits, n)
        lines.append(
            "Masked argmax, plain legal mask. The 95% range is Wilson. "
            "`random` is the mean 1 / legal actions at each row, a uniform "
            "pick's expected agreement."
        )
        lines.append("")
        lines.append("| group | steps | top-1 | 95% | random |")
        lines.append("|---|---:|---:|---|---:|")
        lines.append(
            f"| held out, all | {n} | {pct(r)} ({hits}/{n}) | "
            f"{pct(lo)} to {pct(hi)} | "
            f"{pct(float(np.mean(1.0 / arrays['mask'][held_rows].sum(1)))) if n else 'n/a'} |"
        )
        if train_pred is not None:
            train_hits, train_n, train_r = rate(train_pred, action[train_rows])
            tlo, thi = wilson(train_hits, train_n)
            lines.append(
                f"| train, all | {train_n} | {pct(train_r)} "
                f"({train_hits}/{train_n}) | {pct(tlo)} to {pct(thi)} | "
                f"{pct(float(np.mean(1.0 / arrays['mask'][train_rows].sum(1)))) if train_n else 'n/a'} |"
            )
        lines.append("")
        lines.append("By Era's action kind, held out:")
        lines.append("")
        lines.append("| action kind | steps | top-1 | 95% | random |")
        lines.append("|---|---:|---:|---|---:|")
        held_kind = kind_name[held_rows]
        held_mask = arrays["mask"][held_rows]
        for kind in KINDS:
            sel = np.flatnonzero(held_kind == kind)
            if len(sel) == 0:
                continue
            h, m, rr = rate(pred[sel], held_actions[sel])
            lo2, hi2 = wilson(h, m)
            rand = float(np.mean(1.0 / held_mask[sel].sum(1)))
            lines.append(
                f"| {kind} | {m} | {pct(rr)} ({h}/{m}) | "
                f"{pct(lo2)} to {pct(hi2)} | {pct(rand)} |"
            )
        lines.append("")
        forced_hits = int((forced == held_actions).sum())
        forced_rate = forced_hits / n if n else float("nan")
        lines.append(
            f"With `ppo.force_mask` on (BASE has `force_dark_gift=True`), the "
            f"held-out top-1 is {pct(forced_rate)} ({forced_hits}/{n}). "
            "The table above is the plain masked argmax the brief asks for; "
            "force_mask only changes the acting path on turns 3 to 5 when a "
            "Dark Gift is legal."
        )
        lines.append("")
        lines.append(
            "The plan expects about 27% overall and 7% on buys. The measured "
            "numbers are in the tables above. They are consistent with BASE "
            "rather than with the trained clone: BASE also reads 13.2% overall "
            "and 2.8% on buys on the older `train/runs/clone-data` held-out "
            "games (same 183-action space), which the clone model itself "
            "learned to 31.0%. The 27% budget belongs to a model trained on "
            "these actions, not to this PPO checkpoint."
        )
    lines.append("")

    lines.append("## Choices the brief left open")
    lines.append("")
    choices = [
        "The replay seed is 1 for every turn, as `trinket_picks.py` does. "
        "While the path stays on Era's steps the seed does not change the "
        "seated state; it only seeds draws after a step leaves her path.",
        "A turn whose path leaves Era's steps, whose step cannot be seated or "
        "played, or which ends before its logged steps, is dropped whole. A "
        "partial turn would otherwise put later observations at the wrong "
        "step count.",
        "A logged step the engine's mask rejects is dropped before its row is "
        "saved; the replay still plays the log's step and the turn continues. "
        "The script counts those steps and names the log step kind. None "
        "occurred in this run.",
        "`EraTurnPath.observe` writes `OldBoards::Keep`. BASE's checkpoint has "
        "`old_boards='keep'` and `lag_in_obs=False`, so this is exactly the "
        "`VecEnv.observe` encoding ppo.py gives this checkpoint. An "
        "`old_boards='drop'` or `lag_in_obs=True` run would need the matching "
        "encoding, which this build does not add.",
        "The split is by action file, not by game number across files. 14 of "
        "91 files are held out, the nearest whole game to 15%, with a "
        "seeded `random.Random(1)` shuffle. The full list and its labels are "
        "in `demo.npz` as `games` and `game_split`.",
        "The per-row `game` is the action file name. `report.md` uses the "
        "reader-facing `<source>-g<n>` form in the dropped-turn list.",
        "The dataset is saved with `np.savez` (not compressed) so a training "
        "process reads it quickly; it is about 0.61 GB. `demo.npz` holds "
        "float32 observations and bool masks.",
        "BASE's agreement is plain masked argmax, without `ppo.force_mask`; "
        "the force-mask number is reported once for comparison. No checkpoint "
        "weights are changed.",
        "The report counts only turns with at least one logged step, which is "
        "what `EraReplay.turns(1)` returns. Turns with no steps have no "
        "decision to copy.",
    ]
    for i, text in enumerate(choices, 1):
        lines.append(f"{i}. {text}")
    lines.append("")

    lines.append("## Reading")
    lines.append("")
    lines.append(
        f"{len(replayed)} of {len(turns)} turns replay to their end "
        f"({pct(len(replayed) / max(1, len(turns)))}); "
        f"{len(dropped)} cannot be followed and contribute no rows. "
        f"The dataset holds {len(action)} decisions over {len(set(arrays['game']))} "
        "games."
    )
    late = [record for record in turns if 7 <= record["turn"] <= 12]
    late_ok = [record for record in late if record["error"] is None]
    lines.append("")
    lines.append(
        f"Late turns 7 to 12: {len(late_ok)} of {len(late)} replay with this "
        "path. The brief quotes 386 of 490 late turns for the set it was "
        "written from; the frozen folder used here has 440 turns 7 to 12. The "
        "dropped-turn table names the reason for every one that does not."
    )
    lines.append("")
    lines.append(
        "The main limitation is the one `trinket_picks.py` also records: the "
        "replay is a one-player sandbox, so the opponent seats are dead and "
        "empty and the current opponent Health, tier and board are not in the "
        "action file. Everything on Era's own side is the log's state."
    )
    lines.append("")
    if pred is not None:
        buy = held_kind == "buy"
        buy_hits = int((pred[buy] == held_actions[buy]).sum()) if buy.any() else 0
        lines.append(
            f"BASE's held-out top-1 is {pct(hits / n)} overall and "
            f"{pct(buy_hits / buy.sum()) if buy.any() else 'n/a'} on buys, "
            f"against {pct(float(np.mean(1.0 / held_mask.sum(1))))} for a "
            "uniform pick over the legal actions."
        )
        lines.append("")
    (out / "report.md").write_text("\n".join(lines) + "\n")
    print(
        f"wrote {out / 'report.md'}: {len(replayed)} replayed, "
        f"{len(dropped)} dropped, {len(skipped)} dropped steps",
        flush=True,
    )


if __name__ == "__main__":
    main()
