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
import threading
import time
from concurrent.futures import ThreadPoolExecutor
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
            for name in ("obs.f32", "mask.u8", "meta.tsv", "dom.u8", "header.json"):
                p = self.folder / name
                if p.exists():
                    p.unlink()
        self._truncate()
        # The meta file is the truth after a half-written game.
        lines = (self.folder / "meta.tsv").read_text().splitlines()
        self.n = max(0, len(lines) - 1)
        dom_path = self.folder / "dom.u8"
        if dom_path.exists() and dom_path.stat().st_size > self.n:
            with open(dom_path, "r+b") as f:
                f.truncate(self.n)
        if not dom_path.exists():
            dom_path.write_bytes(b"\0" * self.n)
        elif dom_path.stat().st_size < self.n:
            with open(dom_path, "ab") as f:
                f.write(b"\0" * (self.n - dom_path.stat().st_size))
        self.obs = open(self.folder / "obs.f32", "ab")
        self.mask = open(self.folder / "mask.u8", "ab")
        self.dom = open(dom_path, "ab")
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
            "dominance": True,
        }
        (self.folder / "header.json").write_text(json.dumps(header, indent=1) + "\n")

    def add_game(self, examples: list[tuple], game_id: str,
                 dominating: list[bool] | None = None) -> None:
        """examples: (obs, mask, action, turn, step, kind, place) per decision.
        dominating: one flag per example, a dominating cycle played it."""
        if not examples:
            self.games += 1
            self.write_header(self.n, self.games)
            return
        obs = np.stack([e[0] for e in examples]).astype(np.float32, copy=False)
        mask = np.stack([e[1] for e in examples]).astype(np.uint8, copy=False)
        if dominating is None:
            dominating = [False] * len(examples)
        dom = np.array(dominating, np.uint8)
        obs.tofile(self.obs)
        mask.tofile(self.mask)
        dom.tofile(self.dom)
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
        self.dom.flush()
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
        self.dom.close()
        self.write_header(self.n, self.games)


class BatchNet:
    """One network forward server for a worker's games.

    B game threads call it independently. Requests that arrive within a
    short window are concatenated into one forward, and each caller gets its
    rows back. The network sees the same per-row logits as a one-row call;
    only the batch dimension changes.
    """

    def __init__(self, net, dev, timeout=0.0006, max_batch=8192):
        import torch
        self.net = net
        self.dev = dev
        self.timeout = float(timeout)
        self.max_batch = int(max_batch)
        self.lock = threading.Lock()
        self.cond = threading.Condition(self.lock)
        self.requests = []
        self.results = {}
        self.next_id = 0
        self.stop = False
        self.error = None
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def __call__(self, x):
        with self.cond:
            if self.error is not None:
                raise RuntimeError(self.error)
            i = self.next_id
            self.next_id += 1
            self.requests.append((i, x))
            self.cond.notify_all()
            while i not in self.results:
                if self.error is not None:
                    raise RuntimeError(self.error)
                if self.stop:
                    raise RuntimeError("batch network stopped with a request waiting")
                self.cond.wait()
            return self.results.pop(i)

    def _run(self):
        import torch
        while True:
            with self.cond:
                while not self.requests and not self.stop:
                    self.cond.wait()
                if self.stop and not self.requests:
                    return
                t0 = time.perf_counter()
                while len(self.requests) < self.max_batch:
                    left = self.timeout - (time.perf_counter() - t0)
                    if left <= 0:
                        break
                    self.cond.wait(timeout=left)
                batch = self.requests
                self.requests = []
            xs = [x for _, x in batch]
            try:
                with torch.no_grad():
                    logits, value = self.net(torch.cat(xs, 0))
                sizes = [int(x.shape[0]) for x in xs]
                off = 0
                out = []
                for i, n in zip((i for i, _ in batch), sizes):
                    out.append((i, (logits[off:off + n], value[off:off + n])))
                    off += n
            except Exception as exc:
                with self.cond:
                    self.error = f"batch forward failed: {exc!r}"
                    self.cond.notify_all()
                return
            with self.cond:
                for i, result in out:
                    self.results[i] = result
                self.cond.notify_all()

    def close(self):
        with self.cond:
            self.stop = True
            self.cond.notify_all()
        self.thread.join(timeout=5)


def _plain_action(net, obs, mask, gen, dev):
    """One plain-network action for one row, sampled at temperature 1."""
    import torch
    from eval_search import forced
    from model import masked
    o = torch.from_numpy(obs).to(dev)
    m = torch.from_numpy(mask).to(dev)
    if o.dim() == 1:
        o = o.unsqueeze(0)
    if m.dim() == 1:
        m = m.unsqueeze(0)
    with torch.no_grad():
        logits, _ = net(o)
    m = forced(o, m)
    logits = masked(logits.float().cpu(), m.cpu())
    probs = torch.softmax(logits, -1)
    return int(torch.multinomial(probs, 1, generator=gen).item())


def worker(payload: dict) -> None:
    """One process: play `target - done` games, write every finished game's
    learner decisions and dominance flags to its shard. GPU workers batch
    their games' network calls; CPU workers are separate processes."""
    import torch
    import eval_search

    cfg = payload
    want = cfg["dev"]
    dev = want if want == "cpu" or torch.cuda.is_available() else "cpu"
    if dev == "cpu":
        torch.set_num_threads(max(1, int(cfg.get("cpu_threads", 1))))
    else:
        torch.set_num_threads(1)
    if dev == "cuda" and cfg["vram_gb"] > 0:
        total = torch.cuda.get_device_properties(0).total_memory / 2**30
        torch.cuda.set_per_process_memory_fraction(min(1.0, cfg["vram_gb"] / total), 0)
    ppo.FORCE_DARK = cfg["force_dark_gift"]
    ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = cfg["old_boards"]
    net = load(cfg["ckpt"], dev)
    if cfg.get("fp32"):
        net.bf16 = False
    net.eval()
    w = cfg["worker"]
    shard = Shard(Path(cfg["out"]) / "shards" / f"w{w}", cfg["resume"], cfg["tag"])
    if shard.games >= cfg["target"]:
        shard.close()
        cfg["queue"].put(("done", w, shard.games, shard.n, 0.0, dev))
        return
    batch = max(1, min(cfg["batch"], cfg["target"] - shard.games))
    use_batcher = dev == "cuda" and batch > 1 and cfg.get("batch_net", True)
    proxy = BatchNet(net, dev) if use_batcher else net
    pool = ThreadPoolExecutor(max_workers=batch)
    boards = None
    if cfg["lineup"][bgpy.REAL] > 0:
        boards, unknown = real_boards.load(Path(cfg["real_boards"]))
        if unknown:
            print(f"w{w}: {len(unknown)} unknown real-board minions", flush=True)
    worker_seed = int(cfg["seed"]) + 1_000_003 * w
    env = bgpy.VecEnv(batch, worker_seed, cfg["lineup"],
                      real_boards=boards, old_boards=cfg["old_boards"])
    b = eval_search.Bufs(batch)
    env.kinds(b.kinds)
    names = env.card_names()
    learner_seats = np.nonzero(b.kinds == bgpy.LEARNER)[1]
    policy = cfg["policy"]
    base0 = (worker_seed * (1 << 32)) & ((1 << 64) - 1)
    game_count = [0] * batch
    searchers: list = [None] * batch
    plain_gens: list = [None] * batch

    def new_game(i: int) -> None:
        """A fresh per-game RNG stream, derived from the engine's own game
        seed, so B=1 and B=4 record the same choices for the same games."""
        gs = (base0 + game_count[i] * batch + i) & ((1 << 64) - 1)
        game_count[i] += 1
        plain = torch.Generator(device="cpu")
        plain.manual_seed((gs ^ 0xD1B54A32D192ED03) & 0xFFFF_FFFF)
        plain_gens[i] = plain
        if policy == "search":
            s = Searcher(
                proxy, dev=dev, mode="search", playouts=cfg["playouts"],
                fights=cfg["fights"], fight_weight=cfg["fight_weight"],
                seed=(gs ^ 0x9E3779B97F4A7C15) & 0xFFFF_FFFF,
                replan=cfg["replan"],
                playout_temperature=cfg["playout_temperature"],
                playout_actions=cfg["playout_actions"], margin=cfg["margin"])
            s.names = names
            s.force = eval_search.forced
            searchers[i] = s

    game_no = shard.games + 1
    run_id = cfg["run_id"]
    game_id = [f"{cfg['tag']}-w{w}-{run_id}-g{game_no + i}" for i in range(batch)]
    pending = [[] for _ in range(batch)]
    steps: dict[tuple[int, int], tuple[int, int]] = {}
    places: dict[tuple[int, int], int] = {}
    for i in range(batch):
        new_game(i)
    done, t0 = shard.games, time.time()

    def act_row(i: int):
        seat = int(b.seat[i])
        if policy == "plain" or int(b.kind[i]) != bgpy.LEARNER:
            a = _plain_action(proxy, b.obs[i], b.mask[i], plain_gens[i], dev)
            return a, False
        s = searchers[i]
        a = int(s.decide(env, i, seat))
        return a, bool(s.last_dominating)

    try:
        while done < cfg["target"]:
            env.observe(b.obs, b.mask, b.seat, b.kind)
            result = list(pool.map(act_row, range(batch)))
            actions = np.array([r[0] for r in result], np.int64)
            doms = [r[1] for r in result]
            for i in range(batch):
                a = int(actions[i])
                if not b.mask[i, a]:
                    legal = [int(x) for x in np.nonzero(b.mask[i])[0][:20]]
                    raise RuntimeError(
                        f"{game_id[i]} seat {int(b.seat[i])}: chosen illegal action {a} "
                        f"({KINDS[KIND_OF[a]]}) in the observation's mask; legal {legal}")
                if int(b.kind[i]) != bgpy.LEARNER:
                    continue
                seat = int(b.seat[i])
                turn = int(b.obs[i, bgpy.SCALAR_TURN])
                last_turn, step = steps.get((i, seat), (turn, 0))
                step = 0 if last_turn != turn else step
                steps[(i, seat)] = (turn, step + 1)
                pending[i].append((
                    b.obs[i].copy(), b.mask[i].copy(), a, turn, step,
                    KINDS[KIND_OF[a]], seat, bool(doms[i]),
                ))
            env.step(actions, b.places, b.tier_turns, b.tribes, b.reset)
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
                                for (o, m, a, turn, step, kind, seat, _) in pending[i]]
                    flags = [d for (*_, d) in pending[i]]
                    shard.add_game(examples, game_id[i], flags)
                    done += 1
                pending[i] = []
                steps = {k: v for k, v in steps.items() if k[0] != i}
                places = {k: v for k, v in places.items() if k[0] != i}
                game_no += 1
                game_id[i] = f"{cfg['tag']}-w{w}-{run_id}-g{game_no}"
            if resets.size:
                env.kinds(b.kinds)
                learner_seats = np.nonzero(b.kinds == bgpy.LEARNER)[1]
                for i in resets:
                    new_game(int(i))
            if done < cfg["target"] and done > shard.games:
                if (done - shard.games) % 10 == 0 or done == cfg["target"]:
                    elapsed = max(time.time() - t0, 1e-9)
                    cfg["queue"].put(("progress", w, done, cfg["target"],
                                      (done - shard.games) / elapsed * 3600.0, dev))
    finally:
        pool.shutdown(wait=True)
        if use_batcher:
            proxy.close()
        shard.close()
    elapsed = max(time.time() - t0, 1e-9)
    cfg["queue"].put(("done", w, done, shard.n,
                      (done - shard.games) / elapsed * 3600.0, dev))


def merge(out: Path, workers: int, shard_dir: Path | None = None) -> dict:
    """Concatenate the worker shards into the recording folder `out`."""
    out = Path(out)
    shard_dir = Path(shard_dir) if shard_dir is not None else out / "shards"
    obs_path, mask_path, meta_path = out / "obs.f32", out / "mask.u8", out / "meta.tsv"
    dom_path = out / "dom.u8"
    obs_tmp = out / "obs.f32.tmp"
    mask_tmp = out / "mask.u8.tmp"
    meta_tmp = out / "meta.tsv.tmp"
    dom_tmp = out / "dom.u8.tmp"
    total = games = 0
    with open(obs_tmp, "wb") as fo, open(mask_tmp, "wb") as fm, \
            open(meta_tmp, "w", newline="") as ftmp, open(dom_tmp, "wb") as fd:
        ftmp.write("\t".join(META_COLS) + "\n")
        for w in range(workers):
            folder = shard_dir / f"w{w}"
            hp = folder / "header.json"
            if not hp.exists():
                continue
            header = json.loads(hp.read_text())
            c, g = int(header["count"]), int(header["games"])
            if c * bgpy.OBS_SIZE * 4 != (folder / "obs.f32").stat().st_size:
                raise SystemExit(f"{folder}: obs.f32 disagrees with header count {c}")
            if c * bgpy.N_ACTIONS != (folder / "mask.u8").stat().st_size:
                raise SystemExit(f"{folder}: mask.u8 disagrees with header count {c}")
            dom_file = folder / "dom.u8"
            if not dom_file.exists():
                fd.write(b"\0" * c)
            else:
                dom = dom_file.read_bytes()
                if len(dom) < c:
                    dom += b"\0" * (c - len(dom))
                fd.write(dom[:c])
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
    os.replace(dom_tmp, dom_path)
    header = {
        "obs_size": bgpy.OBS_SIZE,
        "n_actions": bgpy.N_ACTIONS,
        "count": total,
        "games": games,
        "git": git_head(),
        "skipped": {},
        "source": "search_games",
        "dominance": True,
    }
    (out / "header.json").write_text(json.dumps(header, indent=1) + "\n")
    return header


def args():
    a = argparse.ArgumentParser()
    a.add_argument("--games", type=int, default=1000, help="finished games to record")
    a.add_argument("--workers", type=int, default=8, help="GPU worker processes")
    a.add_argument("--batch", type=int, default=2, help="games per GPU worker in flight")
    a.add_argument("--cpu-workers", type=int, default=0,
                   help="extra worker processes with the network on the CPU")
    a.add_argument("--cpu-threads", type=int, default=1, help="torch threads per CPU worker")
    a.add_argument("--cpu-batch", type=int, default=1, help="games per CPU worker in flight")
    a.add_argument("--gpu-weight", type=float, default=0.0,
                   help="target share of a GPU worker relative to a CPU worker "
                        "(0: its batch size)")
    a.add_argument("--cpu-weight", type=float, default=1.0,
                   help="target share of a CPU worker")
    a.add_argument("--batch-net", action=argparse.BooleanOptionalAction, default=True,
                   help="batch a GPU worker's game threads into one network forward")
    a.add_argument("--fp32", action="store_true",
                   help="run the network without bf16 autocast (debugging batch equality)")
    a.add_argument("--policy", choices=["search", "plain"], default="search",
                   help="teacher: the search, or the plain network for plain games")
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
    gpu_workers = max(1, min(cfg.workers, cfg.games))
    cpu_workers = max(0, min(cfg.cpu_workers, cfg.games - gpu_workers))
    workers = gpu_workers + cpu_workers
    gpu_weight = cfg.gpu_weight if cfg.gpu_weight > 0 else float(cfg.batch)
    weights = [gpu_weight] * gpu_workers + [cfg.cpu_weight] * cpu_workers
    raw = [cfg.games * x / sum(weights) for x in weights]
    targets = [int(x) for x in raw]
    for i in sorted(range(workers), key=lambda i: raw[i] - targets[i], reverse=True)[:cfg.games - sum(targets)]:
        targets[i] += 1
    ctx = mp.get_context("spawn")
    q = ctx.Queue()
    procs = []
    start = time.time()
    run_id = f"{int(start)}"
    for w in range(workers):
        target = targets[w]
        dev = "cuda" if w < gpu_workers else "cpu"
        payload = {
            "worker": w, "target": target,
            "batch": cfg.batch if dev == "cuda" else cfg.cpu_batch,
            "dev": dev, "cpu_threads": cfg.cpu_threads,
            "policy": cfg.policy, "ckpt": cfg.ckpt,
            "batch_net": cfg.batch_net, "fp32": cfg.fp32,
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
    print(f"search games: {cfg.games} games over {gpu_workers} GPU workers "
          f"(batch {cfg.batch}) and {cpu_workers} CPU workers "
          f"(batch {cfg.cpu_batch}, {cfg.cpu_threads} threads), policy {cfg.policy}, "
          f"{cfg.ckpt}, out {out}", flush=True)
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
            _, w, d, target, rate, dev = msg
            done[w], rates[w] = d, rate
            print(f"  w{w} ({dev}): {d}/{target} games, {rate:.1f} games/hour", flush=True)
        elif msg[0] == "done":
            _, w, d, n, rate, dev = msg
            done[w], examples[w], rates[w] = d, n, rate
            print(f"  w{w} ({dev}) done: {d} games, {n} examples, "
                  f"{rate:.1f} games/hour", flush=True)
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
        "gpu_workers": gpu_workers,
        "cpu_workers": cpu_workers,
        "batch": cfg.batch,
        "cpu_batch": cfg.cpu_batch,
        "cpu_threads": cfg.cpu_threads,
        "policy": cfg.policy,
        "target": cfg.games,
        "ckpt": str(cfg.ckpt),
        "seed": cfg.seed,
        "source": "search_games",
        "dominance": True,
    }
    (out / "search_games.json").write_text(json.dumps(summary, indent=1) + "\n")
    print(f"wrote {total_games} games, {header['count']} examples to {out} in "
          f"{elapsed / 60:.1f} min ({summary['games_per_hour']:.1f} games/hour, "
          f"{workers} workers)", flush=True)


if __name__ == "__main__":
    main()
