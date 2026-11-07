"""Play training games with the search as the teacher (search v3, step 1).

The searching seats are the two learner seats of the training lineup
(2 learners, 2 snapshots, 4 real boards). Every decision they make is written
in the same format `bgbench record` writes, so train/imitate.py reads the
folder without changes:

    .venv/bin/python train/search_games.py --games 400 --workers 8

The search is search v1 with its current defaults (fight weight 1, replan
every, one greedy playout, 60 playout actions, margin 0, the forced mask).
The snapshot seats are played by the same checkpoint without search; hero,
trinket and Dark Gift picks stay the network's, as in search v1.

Parallel workers each own a batch of games and write a shard under
<out>/shards/w<k>/ in the same recording format. The parent merges the shards
into <out> when they finish. A shard is written incrementally, one game at a
time, and --resume (the default) continues from the games it already holds,
so a crash loses at most the games in flight.
"""

from __future__ import annotations

import argparse
import json
import multiprocessing as mp
import os
import queue
import subprocess
import time
from pathlib import Path

import numpy as np

import bgpy
import ppo
import real_boards
from model import load
from search import KIND_OF, KINDS, Searcher

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CKPT = ROOT / "train" / "runs" / "r1106-keep" / "ckpt.pt"
DEFAULT_OUT = ROOT / "train" / "runs" / "search-games"
LINEUP = [2, 2, 0, 0, 0, 4]
META_COLS = ["i", "action", "game", "turn", "step", "kind", "place", "patch"]


def git_head() -> str:
    try:
        return subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, check=True,
                              capture_output=True, text=True).stdout.strip()
    except Exception:
        return "unknown"


class Shard:
    """One worker's recording: obs.f32, mask.u8, meta.tsv and header.json,
    appended one finished game at a time. --resume truncates a half-written
    game back to the last header count."""

    def __init__(self, folder: Path, resume: bool, tag: str):
        self.folder = Path(folder)
        self.folder.mkdir(parents=True, exist_ok=True)
        self.tag = tag
        header_path = self.folder / "header.json"
        self.games = 0
        self.n = 0
        if resume and header_path.exists():
            header = json.loads(header_path.read_text())
            self.games = int(header.get("games", 0))
            self.n = int(header.get("count", 0))
        else:
            for name in ("obs.f32", "mask.u8", "meta.tsv", "header.json"):
                p = self.folder / name
                if p.exists():
                    p.unlink()
        self._truncate()
        # The meta file is the truth after a half-written game.
        lines = (self.folder / "meta.tsv").read_text().splitlines()
        self.n = max(0, len(lines) - 1)
        self.obs = open(self.folder / "obs.f32", "ab")
        self.mask = open(self.folder / "mask.u8", "ab")
        self.meta = open(self.folder / "meta.tsv", "a", newline="")
        if self.meta.tell() == 0:
            self.meta.write("\t".join(META_COLS) + "\n")
        self.write_header(self.n, self.games)

    def _truncate(self) -> None:
        obs_path = self.folder / "obs.f32"
        mask_path = self.folder / "mask.u8"
        meta_path = self.folder / "meta.tsv"
        n = self.n
        good_obs = n * bgpy.OBS_SIZE
        good_mask = n * bgpy.N_ACTIONS
        if obs_path.exists() and obs_path.stat().st_size > good_obs * 4:
            with open(obs_path, "r+b") as f:
                f.truncate(good_obs * 4)
        elif not obs_path.exists():
            obs_path.touch()
        if mask_path.exists() and mask_path.stat().st_size > good_mask:
            with open(mask_path, "r+b") as f:
                f.truncate(good_mask)
        elif not mask_path.exists():
            mask_path.touch()
        lines = []
        if meta_path.exists():
            lines = meta_path.read_text().splitlines()
        if not lines:
            lines = ["\t".join(META_COLS)]
        lines = lines[:n + 1]
        while len(lines) < n + 1:
            # A header count past the file: drop back to what meta really has.
            break
        if len(lines) != n + 1:
            self.n = len(lines) - 1
            good_obs = self.n * bgpy.OBS_SIZE
            good_mask = self.n * bgpy.N_ACTIONS
            with open(obs_path, "r+b") as f:
                f.truncate(good_obs * 4)
            with open(mask_path, "r+b") as f:
                f.truncate(good_mask)
        meta_path.write_text("\n".join(lines) + "\n")

    def write_header(self, count: int, games: int) -> None:
        header = {
            "obs_size": bgpy.OBS_SIZE,
            "n_actions": bgpy.N_ACTIONS,
            "count": int(count),
            "games": int(games),
            "git": git_head(),
            "skipped": {},
            "source": "search_games",
            "tag": self.tag,
        }
        (self.folder / "header.json").write_text(json.dumps(header, indent=1) + "\n")

    def add_game(self, examples: list[tuple], game_id: str) -> None:
        """examples: (obs, mask, action, turn, step, kind, place) per decision."""
        if not examples:
            self.games += 1
            self.write_header(self.n, self.games)
            return
        obs = np.stack([e[0] for e in examples]).astype(np.float32, copy=False)
        mask = np.stack([e[1] for e in examples]).astype(np.uint8, copy=False)
        obs.tofile(self.obs)
        mask.tofile(self.mask)
        for j, e in enumerate(examples):
            self.meta.write("\t".join([
                str(self.n + j), str(int(e[2])), game_id, str(int(e[3])), str(int(e[4])),
                str(e[5]), str(int(e[6])), "selfplay",
            ]) + "\n")
        self.n += len(examples)
        self.games += 1
        self.meta.flush()
        self.obs.flush()
        self.mask.flush()
        self.write_header(self.n, self.games)

    @property
    def n(self) -> int:
        return getattr(self, "_n", 0)

    @n.setter
    def n(self, value: int) -> None:
        self._n = int(value)

    def close(self) -> None:
        self.meta.close()
        self.obs.close()
        self.mask.close()
        self.write_header(self.n, self.games)


def _plain_sample(net, obs, mask, gen, dev):
    import torch
    from eval_search import forced
    from model import masked
    o = torch.from_numpy(obs).to(dev)
    m = torch.from_numpy(mask).to(dev)
    with torch.no_grad():
        logits, _ = net(o)
    m = forced(o, m)
    logits = masked(logits.float().cpu(), m.cpu())
    probs = torch.softmax(logits, -1)
    return torch.multinomial(probs, 1, generator=gen).squeeze(1).numpy()


def worker(payload: dict) -> None:
    """One process: play `target - done` games with its own batch, append
    every finished searching seat's decision to its shard."""
    import torch
    import eval_search

    torch.set_num_threads(1)
    cfg = payload
    dev = "cuda" if torch.cuda.is_available() else "cpu"
    if dev == "cuda" and cfg["vram_gb"] > 0:
        total = torch.cuda.get_device_properties(0).total_memory / 2**30
        torch.cuda.set_per_process_memory_fraction(min(1.0, cfg["vram_gb"] / total), 0)
    ppo.FORCE_DARK = cfg["force_dark_gift"]
    ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = cfg["old_boards"]
    net = load(cfg["ckpt"], dev)
    net.eval()
    gen = torch.Generator(device="cpu")
    gen.manual_seed((int(cfg["seed"]) + 17 * int(cfg["worker"])) & 0xFFFF_FFFF)
    boards = None
    if cfg["lineup"][bgpy.REAL] > 0:
        boards, unknown = real_boards.load(Path(cfg["real_boards"]))
        if unknown:
            print(f"w{cfg['worker']}: {len(unknown)} unknown real-board minions", flush=True)
    w = cfg["worker"]
    shard = Shard(Path(cfg["out"]) / "shards" / f"w{w}", cfg["resume"], cfg["tag"])
    if shard.games >= cfg["target"]:
        shard.close()
        cfg["queue"].put(("done", w, shard.games, shard.n, 0.0))
        return
    batch = max(1, min(cfg["batch"], cfg["target"] - shard.games))
    env = bgpy.VecEnv(batch, int(cfg["seed"]) + 1_000_003 * w, cfg["lineup"],
                      real_boards=boards, old_boards=cfg["old_boards"])
    b = eval_search.Bufs(batch)
    env.kinds(b.kinds)
    names = env.card_names()
    searcher = Searcher(
        net, dev=dev, mode="search", playouts=cfg["playouts"], fights=cfg["fights"],
        fight_weight=cfg["fight_weight"], seed=cfg["seed"] + 7919 * w,
        replan=cfg["replan"], playout_temperature=cfg["playout_temperature"],
        playout_actions=cfg["playout_actions"], margin=cfg["margin"])
    searcher.names = names
    searcher.force = eval_search.forced
    learner_seats = np.nonzero(b.kinds == bgpy.LEARNER)[1]
    game_no = shard.games + 1
    run_id = cfg["run_id"]
    game_id = [f"{cfg['tag']}-w{w}-{run_id}-g{game_no + i}" for i in range(batch)]
    pending = [[] for _ in range(batch)]
    steps: dict[tuple[int, int], tuple[int, int]] = {}
    # Placement is reported once, when a seat is knocked out (and for the
    # winner when the game ends), not again at the reset: keep it.
    places: dict[tuple[int, int], int] = {}
    done, t0 = shard.games, time.time()
    try:
        while done < cfg["target"]:
            env.observe(b.obs, b.mask, b.seat, b.kind)
            actions = np.zeros(batch, np.int64)
            for i in range(batch):
                seat = int(b.seat[i])
                if int(b.kind[i]) != bgpy.LEARNER:
                    continue
                a = int(searcher.decide(env, i, seat))
                actions[i] = a
                if not b.mask[i, a]:
                    legal = [int(x) for x in np.nonzero(b.mask[i])[0][:20]]
                    raise RuntimeError(
                        f"{game_id[i]} seat {seat}: the search chose illegal action {a} "
                        f"({KINDS[KIND_OF[a]]}) in the observation's mask; legal {legal}; "
                        f"plans {searcher.plans}")
                turn = int(b.obs[i, bgpy.SCALAR_TURN])
                last_turn, step = steps.get((i, seat), (turn, 0))
                step = 0 if last_turn != turn else step
                steps[(i, seat)] = (turn, step + 1)
                pending[i].append((
                    b.obs[i].copy(), b.mask[i].copy(), a, turn, step,
                    KINDS[KIND_OF[a]], int(seat),
                ))
            snap = b.kind != bgpy.LEARNER
            if snap.any():
                actions[snap] = eval_search.sample(
                    net, b.obs[snap], b.mask[snap], gen, dev)
            try:
                env.step(actions, b.places, b.tier_turns, b.tribes, b.reset)
            except Exception as exc:
                rows = []
                for i in range(batch):
                    a = int(actions[i])
                    rows.append((i, int(b.seat[i]), int(b.obs[i, bgpy.SCALAR_TURN]),
                                 a, KINDS[KIND_OF[a]], bool(b.mask[i, a])))
                raise RuntimeError(
                    f"{game_id[0]} step failed: {exc}; rows {rows}; "
                    f"plans { {k: v for k, v in searcher.plans.items()} }") from exc
            for e, s in zip(*np.nonzero(b.places)):
                places[(int(e), int(s))] = int(b.places[e, s])
            resets = np.nonzero(b.reset)[0]
            for i in resets:
                i = int(i)
                if done < cfg["target"]:
                    place_of_seat = {int(s): places.get((i, int(s)), 0)
                                     for s in learner_seats}
                    bad = [s for s, p in place_of_seat.items() if not 1 <= p <= 8]
                    if bad:
                        raise RuntimeError(f"game {game_id[i]} has no placement for seats {bad}")
                    examples = [(o, m, a, turn, step, kind, place_of_seat[seat])
                                for (o, m, a, turn, step, kind, seat) in pending[i]]
                    shard.add_game(examples, game_id[i])
                    done += 1
                pending[i] = []
                steps = {k: v for k, v in steps.items() if k[0] != i}
                places = {k: v for k, v in places.items() if k[0] != i}
                game_no += 1
                game_id[i] = f"{cfg['tag']}-w{w}-{run_id}-g{game_no}"
            if resets.size:
                for i in resets:
                    for seat in learner_seats:
                        searcher.plans.pop((int(i), int(seat)), None)
                        searcher._judged.pop((int(i), int(seat)), None)
                # The lineup is shuffled into new seats every game.
                env.kinds(b.kinds)
                learner_seats = np.nonzero(b.kinds == bgpy.LEARNER)[1]
            if done < cfg["target"] and done > shard.games:
                if (done - shard.games) % 10 == 0 or done == cfg["target"]:
                    elapsed = max(time.time() - t0, 1e-9)
                    cfg["queue"].put(("progress", w, done, cfg["target"],
                                      (done - shard.games) / elapsed * 3600.0))
    finally:
        shard.close()
    elapsed = max(time.time() - t0, 1e-9)
    cfg["queue"].put(("done", w, done, shard.n, (done - shard.games) / elapsed * 3600.0))


def merge(out: Path, workers: int) -> dict:
    """Concatenate the worker shards into the recording folder `out`."""
    out = Path(out)
    obs_path, mask_path, meta_path = out / "obs.f32", out / "mask.u8", out / "meta.tsv"
    obs_tmp = out / "obs.f32.tmp"
    mask_tmp = out / "mask.u8.tmp"
    meta_tmp = out / "meta.tsv.tmp"
    total = games = 0
    with open(obs_tmp, "wb") as fo, open(mask_tmp, "wb") as fm, \
            open(meta_tmp, "w", newline="") as ftmp:
        ftmp.write("\t".join(META_COLS) + "\n")
        for w in range(workers):
            folder = out / "shards" / f"w{w}"
            hp = folder / "header.json"
            if not hp.exists():
                continue
            header = json.loads(hp.read_text())
            c, g = int(header["count"]), int(header["games"])
            if c * bgpy.OBS_SIZE * 4 != (folder / "obs.f32").stat().st_size:
                raise SystemExit(f"{folder}: obs.f32 disagrees with header count {c}")
            if c * bgpy.N_ACTIONS != (folder / "mask.u8").stat().st_size:
                raise SystemExit(f"{folder}: mask.u8 disagrees with header count {c}")
            with open(folder / "obs.f32", "rb") as f:
                while True:
                    chunk = f.read(1 << 24)
                    if not chunk:
                        break
                    fo.write(chunk)
            with open(folder / "mask.u8", "rb") as f:
                while True:
                    chunk = f.read(1 << 24)
                    if not chunk:
                        break
                    fm.write(chunk)
            lines = (folder / "meta.tsv").read_text().splitlines()[1:]
            if len(lines) != c:
                raise SystemExit(f"{folder}: meta.tsv has {len(lines)} rows, header says {c}")
            for line in lines:
                cells = line.split("\t")
                cells[0] = str(total)
                ftmp.write("\t".join(cells) + "\n")
                total += 1
            games += g
    os.replace(obs_tmp, obs_path)
    os.replace(mask_tmp, mask_path)
    os.replace(meta_tmp, meta_path)
    header = {
        "obs_size": bgpy.OBS_SIZE,
        "n_actions": bgpy.N_ACTIONS,
        "count": total,
        "games": games,
        "git": git_head(),
        "skipped": {},
        "source": "search_games",
    }
    (out / "header.json").write_text(json.dumps(header, indent=1) + "\n")
    return header


def args():
    a = argparse.ArgumentParser()
    a.add_argument("--games", type=int, default=1000, help="finished games to record")
    a.add_argument("--workers", type=int, default=8, help="worker processes")
    a.add_argument("--batch", type=int, default=2, help="games per worker in flight")
    a.add_argument("--ckpt", default=str(DEFAULT_CKPT), help="the current network")
    a.add_argument("--out", default=str(DEFAULT_OUT), help="the recording folder")
    a.add_argument("--seed", type=int, default=11)
    a.add_argument("--tag", default="search", help="game-id prefix, unique within a run")
    a.add_argument("--real-boards", default=str(ROOT / "boards" / "real"))
    a.add_argument("--lineup", default=",".join(map(str, LINEUP)))
    a.add_argument("--old-boards", choices=["keep", "drop"], default="keep")
    a.add_argument("--force-dark-gift", action=argparse.BooleanOptionalAction, default=True)
    a.add_argument("--playouts", type=int, default=1)
    a.add_argument("--fights", type=int, default=20)
    a.add_argument("--fight-weight", type=float, default=1.0)
    a.add_argument("--replan", choices=["every", "turn", "cycle"], default="every")
    a.add_argument("--playout-temperature", type=float, default=0.0)
    a.add_argument("--playout-actions", type=int, default=60)
    a.add_argument("--margin", type=float, default=0.0)
    a.add_argument("--vram-gb", type=float, default=0.0,
                   help="cap each worker at this much GPU memory (pcbusy ON: 6)")
    a.add_argument("--resume", action=argparse.BooleanOptionalAction, default=True)
    return a.parse_args()


def main() -> None:
    cfg = args()
    if cfg.games < 1:
        raise SystemExit("--games must be at least 1")
    lineup = [int(x) for x in cfg.lineup.split(",")]
    if len(lineup) != bgpy.SEAT_KINDS or lineup[bgpy.LEARNER] < 1:
        raise SystemExit(f"--lineup needs {bgpy.SEAT_KINDS} numbers with learners")
    out = Path(cfg.out)
    out.mkdir(parents=True, exist_ok=True)
    workers = max(1, min(cfg.workers, cfg.games))
    base, extra = divmod(cfg.games, workers)
    ctx = mp.get_context("spawn")
    q = ctx.Queue()
    procs, targets = [], []
    start = time.time()
    run_id = f"{int(start)}"
    for w in range(workers):
        target = base + (1 if w < extra else 0)
        targets.append(target)
        payload = {
            "worker": w, "target": target, "batch": cfg.batch, "ckpt": cfg.ckpt,
            "run_id": run_id,
            "out": str(out), "seed": cfg.seed, "tag": cfg.tag,
            "lineup": lineup, "real_boards": cfg.real_boards,
            "old_boards": cfg.old_boards, "force_dark_gift": cfg.force_dark_gift,
            "playouts": cfg.playouts, "fights": cfg.fights,
            "fight_weight": cfg.fight_weight, "replan": cfg.replan,
            "playout_temperature": cfg.playout_temperature,
            "playout_actions": cfg.playout_actions, "margin": cfg.margin,
            "vram_gb": cfg.vram_gb, "resume": cfg.resume, "queue": q,
        }
        p = ctx.Process(target=worker, args=(payload,), daemon=False)
        p.start()
        procs.append(p)
    print(f"search games: {cfg.games} games over {workers} workers "
          f"(batch {cfg.batch}), {cfg.ckpt}, out {out}", flush=True)
    done = [0] * workers
    examples = [0] * workers
    rates = [0.0] * workers
    while any(p.is_alive() for p in procs):
        try:
            msg = q.get(timeout=1.0)
        except queue.Empty:
            for p in procs:
                p.join(timeout=0)
            continue
        if msg[0] == "progress":
            _, w, d, target, rate = msg
            done[w], rates[w] = d, rate
            print(f"  w{w}: {d}/{target} games, {rate:.1f} games/hour", flush=True)
        elif msg[0] == "done":
            _, w, d, n, rate = msg
            done[w], examples[w], rates[w] = d, n, rate
            print(f"  w{w} done: {d} games, {n} examples, {rate:.1f} games/hour",
                  flush=True)
    for p in procs:
        p.join()
        if p.exitcode != 0:
            raise SystemExit(f"worker exited with {p.exitcode}")
    # Drain messages that arrived between the last check and the joins.
    while True:
        try:
            msg = q.get_nowait()
        except queue.Empty:
            break
        if msg[0] == "done":
            done[msg[1]], examples[msg[1]] = msg[2], msg[3]
    elapsed = time.time() - start
    header = merge(out, workers)
    total_games = int(header["games"])
    summary = {
        "games": total_games,
        "examples": int(header["count"]),
        "seconds": elapsed,
        "games_per_hour": total_games / max(elapsed, 1e-9) * 3600.0,
        "workers": workers,
        "batch": cfg.batch,
        "target": cfg.games,
        "ckpt": str(cfg.ckpt),
        "seed": cfg.seed,
        "source": "search_games",
    }
    (out / "search_games.json").write_text(json.dumps(summary, indent=1) + "\n")
    print(f"wrote {total_games} games, {header['count']} examples to {out} in "
          f"{elapsed / 60:.1f} min ({summary['games_per_hour']:.1f} games/hour, "
          f"{workers} workers)", flush=True)


if __name__ == "__main__":
    main()
