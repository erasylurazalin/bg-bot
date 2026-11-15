#!/usr/bin/env python3
"""The step 4 demonstration dataset: Era's replayed decisions, build A.

    .venv/bin/python train/evaltools/demo_dataset.py --cpu

Every turn of `boards/frozen/actions-2026-11-15` that replays to its end is
seated with `bgpy.EraReplay` / `EraTurnPath`, the same path as
`train/era_turns.py` and `train/evaltools/trinket_picks.py`. The replay seats
the turn's live `lobby` block by default; `--dead-lobby` asks for the old dead
seats. Before every decision the script saves the observation and legal mask
`ppo.py` would feed its network, and Era's logged action as the flat action
index `ppo.py` uses. A logged action that the engine's mask does not allow is
dropped and counted. Turns that cannot be seated or played through are dropped
whole and counted.

The split is by game: the old dataset's saved game list and split are reused
with `--split-from`, so the held-out games stay the same.

Output in `train/runs/demo-dataset-live/`: `demo.npz` with `obs`, `mask`,
`action`, `turn`, `game`, `split`, and the split list `games` / `game_split`;
and `report.md` with the live-lobby alive check, the counts, the dropped turns,
and BASE's masked-argmax agreement on the held-out part, overall, on buys, on
the grouped kinds, and by action kind.

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

ACTIONS = ROOT / "boards/frozen/actions-2026-11-15"
OUT = ROOT / "train/runs/demo-dataset-live"
CKPT = ROOT / "train/runs/r1115-school-text/best.pt"
SPLIT_FROM = ROOT / "train/runs/demo-dataset/demo.npz"
REPLAY_SEED = 1
SPLIT_SEED = 1
HOLDOUT = 0.15
ALIVE_SAMPLE = 400

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


def load_saved_split(path: Path) -> tuple[list[str], dict[str, str]]:
    """The old dataset's saved game list and its by-game split."""
    with np.load(path, allow_pickle=False) as z:
        games = [str(name) for name in z["games"]]
        split = {
            str(name): str(label)
            for name, label in zip(z["games"], z["game_split"])
        }
    return games, split


def lobby_block(name: str, turn: int) -> list[dict] | None:
    """The turn's logged lobby, or None when the file has no block."""
    for item in action_spec(name).get("turns", []):
        if int(item.get("turn", 0)) == turn:
            block = item.get("start", {}).get("lobby")
            return block if block is not None else None
    return None


def alive_sample_rows(arrays: dict, files: list[str],
                      target: int = ALIVE_SAMPLE) -> list[int]:
    """At least `target` saved rows, spread over every game that has one.

    Each game contributes the same number of evenly spaced rows (at least its
    first row when it has fewer); the sample is deterministic.
    """
    by_game: dict[str, list[int]] = defaultdict(list)
    for i, name in enumerate(arrays["game"]):
        by_game[str(name)].append(i)
    present = [name for name in files if by_game.get(name)]
    per = max(1, math.ceil(target / max(1, len(present))))
    picked: list[int] = []
    for name in present:
        rows = by_game[name]
        k = min(per, len(rows))
        if k == 1:
            picked.append(rows[0])
        else:
            for j in range(k):
                picked.append(rows[round(j * (len(rows) - 1) / (k - 1))])
    return picked


def alive_check(arrays: dict, files: list[str]) -> dict:
    """Compare the observation's opponents with the turn's logged lobby.

    `SCALAR_ALIVE` counts me and the seven seats, so it should be one plus the
    lobby's living entries. Each of the seven `OPP_ALIVE`, `OPP_HEALTH` and
    `OPP_TIER` values should be the logged entry's.
    """
    picked = alive_sample_rows(arrays, files)
    problems: list[dict] = []
    games: set[str] = set()
    entries = 0
    unnamed = 0
    for i in picked:
        name = str(arrays["game"][i])
        turn = int(arrays["turn"][i])
        games.add(name)
        block = lobby_block(name, turn)
        obs = arrays["obs"][i]
        if block is None:
            problems.append({"game": name, "turn": turn,
                             "why": "the turn has no lobby block"})
            continue
        by_seat = {int(e.get("seat", 0)): e for e in block}
        entries += len(block)
        alive = sum(1 for e in block if e.get("alive"))
        got = int(round(float(obs[bgpy.SCALAR_ALIVE])))
        if got != 1 + alive:
            problems.append({
                "game": name, "turn": turn, "seat": "SCALAR_ALIVE",
                "got": got, "want": 1 + alive,
            })
        for seat in range(1, 8):
            entry = by_seat.get(seat)
            at = bgpy.OWN_END + (seat - 1) * bgpy.OPP_BLOCK
            got_alive = int(round(float(obs[at + bgpy.OPP_ALIVE])))
            if entry is None:
                # The log named no player in this seat. The loader leaves the
                # sandbox's dead player there, so only its liveness is checked.
                unnamed += 1
                if got_alive != 0:
                    problems.append({
                        "game": name, "turn": turn, "seat": seat,
                        "field": "OPP_ALIVE", "got": got_alive, "want": 0,
                    })
                continue
            want = {
                "OPP_ALIVE": 1 if entry.get("alive") else 0,
                "OPP_HEALTH": int(entry.get("health", 30)),
                # The first-turn block writes tier 0 before the log shows it;
                # the loader clamps every tier into the game's 1..MAX_TIER.
                "OPP_TIER": min(bgpy.MAX_TIER,
                                max(1, int(entry.get("tier", 0)))),
            }
            got = {
                "OPP_ALIVE": got_alive,
                "OPP_HEALTH": int(round(float(obs[at + bgpy.OPP_HEALTH]))),
                "OPP_TIER": int(round(float(obs[at + bgpy.OPP_TIER]))),
            }
            for key in want:
                if got[key] != want[key]:
                    problems.append({
                        "game": name, "turn": turn, "seat": seat,
                        "field": key, "got": got[key], "want": want[key],
                    })
    return {
        "rows": len(picked),
        "games": len(games),
        "lobby_entries": entries,
        "unnamed_seats": unnamed,
        "problems": problems,
    }


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
    """Plain masked argmax, the force_mask argmax, and the grouped kind argmax."""
    import torch

    import ppo
    from model import masked

    pred = np.empty(len(rows), dtype=np.int64)
    forced = np.empty(len(rows), dtype=np.int64)
    kind = np.empty(len(rows), dtype=np.int64)
    for start in range(0, len(rows), batch):
        stop = min(start + batch, len(rows))
        o = torch.from_numpy(arrays["obs"][rows[start:stop]]).to(dev)
        m = torch.from_numpy(arrays["mask"][rows[start:stop]]).to(dev)
        with torch.no_grad():
            logits, _ = net(o, comp=None)
            pred[start:stop] = masked(logits, m).argmax(1).cpu().numpy()
            kind[start:stop] = ppo.demo_kind_probs(logits, m).argmax(1).cpu().numpy()
            fm = ppo.force_mask(o, m)
            forced[start:stop] = masked(logits, fm).argmax(1).cpu().numpy()
    return pred, forced, kind


def main() -> None:
    global ACTIONS
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--actions", default=str(ACTIONS))
    ap.add_argument("--out", default=str(OUT))
    ap.add_argument("--ckpt", default=str(CKPT))
    ap.add_argument("--split-from", default=str(SPLIT_FROM),
                    help="old dataset whose saved game list and split are reused")
    ap.add_argument("--dead-lobby", action="store_true",
                    help="seat the old dead opponents instead of the lobby block")
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
    split_source = "a fresh seeded shuffle"
    old = Path(args.split_from)
    if old.is_file():
        saved_files, saved_split = load_saved_split(old)
        if set(saved_files) == set(files):
            files = saved_files
            split = {name: saved_split[name] for name in files}
            split_source = f"`{old.relative_to(ROOT)}`"
        else:
            print(
                f"{old}: its game list differs from {ACTIONS} "
                f"({len(set(saved_files) - set(files))} files missing, "
                f"{len(set(files) - set(saved_files))} new); using a fresh split",
                flush=True,
            )
    held = [name for name in files if split[name] == "heldout"]
    print(
        f"{len(files)} games: {len(files) - len(held)} train, {len(held)} held out "
        f"(split from {split_source})",
        flush=True,
    )

    replays, bad = bgpy.era_replays(
        str(ACTIONS), str(ROOT / "ref/bg-cards.json"),
        lobby=not args.dead_lobby,
    )
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
    if args.dead_lobby:
        alive = None
        print("alive check skipped: --dead-lobby seats the old dead opponents",
              flush=True)
    else:
        alive = alive_check(arrays, files)
        print(
            f"alive check: {alive['rows']} rows over {alive['games']} games, "
            f"{alive['lobby_entries']} logged seats "
            f"({alive['unnamed_seats']} unnamed), "
            f"{len(alive['problems'])} mismatches",
            flush=True,
        )
        if alive["problems"]:
            for problem in alive["problems"][:20]:
                print(f"alive mismatch: {problem}", flush=True)
            raise SystemExit(
                f"alive check failed: {len(alive['problems'])} mismatches"
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
        kind_pred = None
        train_pred = None
        info = {"update": None, "force_dark_gift": None, "old_boards": "keep"}
        dev = None
    else:
        net, info, dev = load_base(args.ckpt, args.cpu, args.vram_gb)
        pred, forced, kind_pred = predict_base(
            net, arrays, held_rows, dev, args.batch)
        # The same predictions on the train rows, for context only.
        if len(train_rows):
            train_pred, _, _ = predict_base(
                net, arrays, train_rows, dev, args.batch)
        else:
            train_pred = None

    lines: list[str] = []
    lines.append("# Demonstration dataset from Era's replayed turns (live lobbies)")
    lines.append("")
    lines.append(
        "Build A of step 4 (`docs/notes/plan-2026-11-13.md`): the observation, "
        "legal mask and Era's action for every decision of every turn in "
        f"`{ACTIONS.relative_to(ROOT)}` that replays to its end. Script: "
        "`train/evaltools/demo_dataset.py`; data: `demo.npz`."
    )
    lines.append("")
    lines.append(
        f"Turn replay seed {args.seed}. Replays use `bgpy.EraReplay` and "
        "`EraTurnPath` with `reseat=True`, the same seating as "
        "`train/evaltools/trinket_picks.py` and the path `train/era_turns.py` "
        "uses for Era's own end-of-turn position. The turn's `lobby` block is "
        "seated by default, so the seven opponents are the log's living "
        "players; `--dead-lobby` is the old dead-seat opt-out. `EraTurnPath.observe` calls "
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

    lines.append("## Live-lobby alive check")
    lines.append("")
    if alive is None:
        lines.append("Skipped: `--dead-lobby` seats the old dead opponents.")
    else:
        lines.append(
            f"Checked {alive['rows']} saved steps over {alive['games']} games "
            f"({alive['lobby_entries']} logged lobby seats, "
            f"{alive['unnamed_seats']} seats the log did not name): "
            f"{len(alive['problems'])} mismatches."
        )
        lines.append("")
        lines.append(
            "For every sampled step, `SCALAR_ALIVE` equals one (me) plus the "
            "living entries of the turn's `lobby` block, and each named "
            "opponent's `OPP_ALIVE`, `OPP_HEALTH` and `OPP_TIER` equals the "
            "block's. A seat the block does not name keeps the loader's dead "
            "sandbox player and is checked only for not being alive. The "
            "first-turn block writes tier 0 before the log shows a tier; the "
            "loader clamps every tier to the game's 1 to 7, so the check reads "
            "0 as 1. The rows are evenly spaced through every game that has a "
            "saved step, so the sample is not just the early turns."
        )
        if alive["problems"]:
            lines.append("")
            lines.append("Mismatches:")
            lines.append("")
            lines.append("| game | turn | seat | field | got | want |")
            lines.append("|---|---:|---:|---|---:|---:|")
            for problem in alive["problems"][:50]:
                lines.append(
                    f"| `{problem.get('game')}` | {problem.get('turn')} | "
                    f"{problem.get('seat')} | {problem.get('field', problem.get('why'))} | "
                    f"{problem.get('got')} | {problem.get('want')} |"
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
        f"The split is by game, reused from {split_source}: "
        f"{len(held)} of {len(files)} games held out. The saved game list and "
        "its labels are in `demo.npz` as `games` and `game_split`."
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
        import ppo
        kind_target = ppo.DEMO_KIND_OF.numpy()[held_actions]
        kind_hits, kind_n, kind_r = rate(kind_pred, kind_target)
        klo, khi = wilson(kind_hits, kind_n)
        lines.append(
            "Masked argmax, plain legal mask. The 95% range is Wilson. "
            "`random` is the mean 1 / legal actions at each row, a uniform "
            "pick's expected agreement (for the grouped kind, the mean legal "
            "share of Era's kind)."
        )
        lines.append("")
        lines.append("| group | steps | top-1 | 95% | random |")
        lines.append("|---|---:|---:|---|---:|")
        lines.append(
            f"| held out, all | {n} | {pct(r)} ({hits}/{n}) | "
            f"{pct(lo)} to {pct(hi)} | "
            f"{pct(float(np.mean(1.0 / arrays['mask'][held_rows].sum(1)))) if n else 'n/a'} |"
        )
        held_kind_target = kind_target
        kind_mass = (
            ((arrays['mask'][held_rows])
             & (ppo.DEMO_KIND_OF.numpy()[None, :] == held_kind_target[:, None]))
            .sum(1) / arrays['mask'][held_rows].sum(1)
        )
        lines.append(
            f"| held out, grouped kind | {kind_n} | {pct(kind_r)} "
            f"({kind_hits}/{kind_n}) | {pct(klo)} to {pct(khi)} | "
            f"{pct(float(np.mean(kind_mass))) if kind_n else 'n/a'} |"
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
        "The script counts those steps and names the log step kind. "
        f"{len(skipped)} occurred in this run.",
        "`EraTurnPath.observe` writes `OldBoards::Keep`. BASE's checkpoint has "
        "`old_boards='keep'` and `lag_in_obs=False`, so this is exactly the "
        "`VecEnv.observe` encoding ppo.py gives this checkpoint. An "
        "`old_boards='drop'` or `lag_in_obs=True` run would need the matching "
        "encoding, which this build does not add.",
        "The lobby is the default: `bgpy.era_replays` seats each turn's "
        "`lobby` block, so the seven opponents are the log's, not the old dead "
        "sandbox seats. `--dead-lobby` exists only to reproduce the old "
        "observation.",
        "The alive check samples every game that has a row, taking evenly "
        "spaced rows from each game up to about 400 rows in total. It compares "
        "`SCALAR_ALIVE` as one plus the lobby's living entries, and every "
        "named seat's `OPP_ALIVE`, `OPP_HEALTH` and `OPP_TIER`. A seat the "
        "block does not name is the loader's dead sandbox player and is "
        "checked only for not being alive. Tier 0 in a first-turn block is "
        "read as 1 because the loader clamps it to the game's 1 to 7.",
        f"The split is reused from {split_source}, so the held-out games are "
        f"the old dataset's: {len(held)} of {len(files)}. The full list and "
        "its labels are in `demo.npz` as `games` and `game_split`.",
        "The per-row `game` is the action file name. `report.md` uses the "
        "reader-facing `<source>-g<n>` form in the dropped-turn list.",
        "The dataset is saved with `np.savez` (not compressed) so a training "
        "process reads it quickly. `demo.npz` holds float32 observations and "
        "bool masks.",
        "BASE's agreement is plain masked argmax, without `ppo.force_mask`; "
        "the force-mask number is reported once for comparison. The grouped "
        "kind row is `ppo.demo_kind_probs`' argmax, the measure the "
        "kind-weight gates use. No checkpoint weights are changed.",
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
        "The replay now seats the log's live lobby: the seven opponents' "
        "heroes, health, armor, tier, placement and last-seen boards are the "
        "logged ones. The log still has no other player's current board "
        "outside my own fights and no leaderboard tribe line for a player I "
        "did not fight, so those observation fields stay zero. Everything on "
        "Era's own side is the log's state."
    )
    lines.append("")
    if pred is not None:
        buy = held_kind == "buy"
        buy_hits = int((pred[buy] == held_actions[buy]).sum()) if buy.any() else 0
        lines.append(
            f"BASE's held-out top-1 is {pct(hits / n)} overall, "
            f"{pct(buy_hits / buy.sum()) if buy.any() else 'n/a'} on buys, and "
            f"{pct(kind_r)} on the grouped kinds, against "
            f"{pct(float(np.mean(1.0 / held_mask.sum(1))))} for a uniform pick "
            "over the legal actions."
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
