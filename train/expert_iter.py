"""Search v3, the expert-iteration loop (docs/notes/training-plans.md "THE PLAN").

One iteration:

1. train/search_games.py plays the training lineup with the current network,
   both learner seats searching (the teacher), and records their choices.
2. train/imitate.py fits the current network to those choices (weight none)
   and to the seat's final placement (value head), with the previous
   iteration's games mixed in (at most two iterations of data).
3. the new network becomes the current one, and the iteration measures it
   WITHOUT search: 512 games vs 7 real boards, 512 vs 7 standard bots, the
   Blue Shell test, the cycle rate on Era's turns, and the Era-turn benchmark
   (plain and search). It also plays the 40-game placement test WITH search.

    .venv/bin/python train/expert_iter.py --iterations 2

Every stage writes its result to train/runs/xit-<n>/tests.json as soon as it
finishes, and --resume (the default) skips a stage whose result is already
there for the same network. search_games.py and era_turns.py resume their own
output too, so a crash loses little. The starting point is
train/runs/r1106-keep/ckpt.pt; train/runs/xit-0/ holds its baseline.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import shutil
import subprocess
import time
from types import SimpleNamespace
from pathlib import Path

import numpy as np
import torch

import bgpy
import eval_search
import ppo
import real_boards
from model import load
from search import Searcher

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_INIT = ROOT / "train" / "runs" / "r1106-keep" / "ckpt.pt"
PYTHON = str(ROOT / ".venv" / "bin" / "python")
START_PLACE = 5.85
STOP_MARGIN = 0.30


def log(msg: str) -> None:
    print(msg, flush=True)


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def run(cmd: list[str], log_path: Path) -> None:
    """A subprocess, its output both on this terminal and in log_path."""
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log("$ " + " ".join(cmd))
    with open(log_path, "a", buffering=1) as f:
        p = subprocess.Popen(cmd, cwd=ROOT, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, text=True, bufsize=1)
        assert p.stdout is not None
        for line in p.stdout:
            print(line, end="", flush=True)
            f.write(line)
        code = p.wait()
    if code != 0:
        raise SystemExit(f"command failed ({code}): {' '.join(cmd)}")


def read_json(path: Path, default):
    try:
        return json.loads(path.read_text())
    except Exception:
        return default


def write_json(path: Path, value) -> None:
    path.write_text(json.dumps(value, indent=1) + "\n")


def tests_of(folder: Path) -> dict:
    return read_json(folder / "tests.json", {})


def save_tests(folder: Path, tests: dict) -> None:
    tests["updated"] = time.strftime("%Y-%m-%d %H:%M:%S")
    write_json(folder / "tests.json", tests)


def stage(net_sha: str, tests: dict, name: str, folder: Path, fn, force: bool):
    """Run fn once for this network; a saved result is reused on resume."""
    if not force and tests.get("net_sha") == net_sha and name in tests.get("stages", {}):
        log(f"  {name}: cached, {tests['stages'][name]}")
        return tests["stages"][name]
    t0 = time.time()
    value = fn()
    tests.setdefault("stages", {})[name] = value
    save_tests(folder, tests)
    log(f"  {name}: {value} ({time.time() - t0:.1f} s)")
    return value


def ppo_eval(net, dev, boards, cfg, lineup, seed):
    net.eval()
    place, _ = ppo.evaluate(net, cfg.eval_games, lineup, dev, seed, boards)
    return float(place)


def shell_args(cfg):
    return SimpleNamespace(
        games=cfg.shell_games, seed=cfg.seed, playouts=cfg.playouts,
        fights=cfg.fights, fight_weight=cfg.fight_weight, replan=cfg.replan,
        playout_temperature=cfg.playout_temperature,
        playout_actions=cfg.playout_actions, margin=cfg.margin,
        positions=cfg.shell_positions, verbose=False,
    )


def cycle_args(cfg):
    return SimpleNamespace(
        turns=cfg.cycle_turns, from_turn=cfg.from_turn, seed=cfg.seed,
        playouts=cfg.playouts, fights=0, fight_weight=0.0, replan=cfg.replan,
        playout_temperature=cfg.playout_temperature,
        playout_actions=cfg.playout_actions, margin=cfg.margin,
    )


def measure_plain(net, dev, boards, cfg, folder: Path, tests: dict, net_sha: str,
                  force: bool, search40: bool = False, era: bool = False) -> dict:
    """The plain-network tests, each saved before the next one starts."""
    ppo.FORCE_DARK = True
    ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = "keep"
    seed = cfg.seed + 10_000
    stage(net_sha, tests, "vs_real",
          folder, lambda: ppo_eval(net, dev, boards, cfg, [1, 0, 0, 0, 0, 7], seed),
          force)
    stage(net_sha, tests, "vs_std",
          folder, lambda: ppo_eval(net, dev, None, cfg, [1, 0, 7, 0, 0, 0], seed + 1),
          force)
    stage(net_sha, tests, "shell",
          folder, lambda: eval_search.shell_test(net, shell_args(cfg), dev, boards),
          force)
    stage(net_sha, tests, "cycle",
          folder, lambda: eval_search.cycle_rate(net, cycle_args(cfg), dev),
          force)
    if search40:
        def play_search():
            searcher = Searcher(
                net, dev=dev, mode="search", playouts=cfg.playouts, fights=cfg.fights,
                fight_weight=cfg.fight_weight, seed=cfg.seed, replan=cfg.replan,
                playout_temperature=cfg.playout_temperature,
                playout_actions=cfg.playout_actions, margin=cfg.margin)
            rec, buys = eval_search.play(
                net, cfg.search_games, cfg.seed, dev, boards=boards,
                searcher=searcher, gen_seed=cfg.seed, track=False)
            s = eval_search.summarize(rec)
            s.pop("place_ci", None)
            return s
        stage(net_sha, tests, "search40", folder, play_search, force)
    if era:
        out = folder / "era-turns"
        cmd = [PYTHON, str(ROOT / "train" / "era_turns.py"),
               "--ckpt", str(folder / "ckpt.pt"), "--name", folder.name,
               "--sample", str(cfg.era_sample), "--repeats", str(cfg.era_repeats),
               "--players", cfg.era_players, "--seed", str(cfg.seed),
               "--out", str(out)]
        def run_era():
            run(cmd, folder / "era_turns.log")
            return era_summary(out / "turns.tsv")
        stage(net_sha, tests, "era", folder, run_era, force)
    return tests


def era_summary(path: Path) -> dict:
    """Overall rows of era_turns.py's turns.tsv, per player."""
    out = {}
    with open(path, newline="") as f:
        rows = list(csv.DictReader(f, delimiter="\t"))
    era_opp = [float(r["era_opp_w"]) for r in rows if r.get("era_opp_w")]
    out["era_opp"] = float(np.mean(era_opp)) if era_opp else None
    for player in ("plain", "search"):
        rs = [r for r in rows if r["player"] == player]
        def mean(key):
            vals = [float(r[key]) for r in rs if r.get(key) not in ("", None)]
            return float(np.mean(vals)) if vals else None
        out[player] = {
            "turns": len(rs),
            "opp": mean("opp_w"),
            "typ": mean("typ_w"),
            "value": mean("value"),
            "gold": mean("gold"),
            "cycles": mean("cycles"),
            "tier": mean("tier"),
        }
    return out


def shell_rate(shell: dict, player: str, field: str) -> float | None:
    return shell.get(f"{player}_{field}")


def stop_broken(tests: dict, baseline: dict | None) -> tuple[bool, str]:
    """The brief's early stop: clearly worse vs real boards and no Shell or
    cycle gain, which means the loop is broken, not slow."""
    real = tests.get("stages", {}).get("vs_real")
    if real is None:
        return False, ""
    start = (baseline or {}).get("stages", {}).get("vs_real") or START_PLACE
    if real <= start + STOP_MARGIN:
        return False, ""
    sh = tests["stages"].get("shell") or {}
    cy = tests["stages"].get("cycle") or {}
    bs = (baseline or {}).get("stages", {}).get("shell") or {}
    bc = (baseline or {}).get("stages", {}).get("cycle") or {}
    shell_gain = ((sh.get("plain_buy", 0) > bs.get("plain_buy", 0) + 0.02)
                  or (sh.get("plain_sell", 0) > bs.get("plain_sell", 0) + 0.02))
    cycle_gain = cy.get("plain_with", 0) > bc.get("plain_with", 0) + 0.02
    if shell_gain or cycle_gain:
        return False, ""
    return True, (f"vs real {real:.3f} is worse than the start {start:.3f} by more "
                  f"than {STOP_MARGIN}, with no Shell or cycle gain")


def write_report(folder: Path, tests: dict, baseline: dict | None, cfg,
                 note: str = "") -> None:
    st = tests.get("stages", {})
    bst = (baseline or {}).get("stages", {})
    lines = [
        f"# Expert iteration: {folder.name}",
        "",
        f"- network: `{folder / 'ckpt.pt'}`",
        f"- starting network: `{cfg.init_from}`",
    ]
    if note:
        lines += ["", f"**{note}**"]
    lines += ["", "## The loop", "",
              f"- search games: {tests.get('search', {})}",
              f"- imitate: {tests.get('imitate', {})}", ""]
    lines += ["## Plain network, without search", "",
              "| test | start | this iteration |",
              "|---|---|---|"]
    def row(name, a, b):
        lines.append(f"| {name} | {fmt(a)} | {fmt(b)} |")
    row("vs 7 real boards (place)", bst.get("vs_real"), st.get("vs_real"))
    row("vs 7 standard bots (place)", bst.get("vs_std"), st.get("vs_std"))
    bs, cs = bst.get("shell") or {}, st.get("shell") or {}
    row("Blue Shell bought %", 100 * bs.get("plain_buy", 0) if bs else None,
        100 * cs.get("plain_buy", 0) if cs else None)
    row("Blue Shell bought and sold %", 100 * bs.get("plain_sell", 0) if bs else None,
        100 * cs.get("plain_sell", 0) if cs else None)
    bc, cc = bst.get("cycle") or {}, st.get("cycle") or {}
    row("Era turns with a cycle %", 100 * bc.get("plain_with", 0) if bc else None,
        100 * cc.get("plain_with", 0) if cc else None)
    row("cycles a turn", bc.get("plain_cycles"), cc.get("plain_cycles"))
    be, ce = bst.get("era") or {}, st.get("era") or {}
    for key, label in (("plain", "Era-turn plain"), ("search", "Era-turn search")):
        p = ce.get(key) or {}
        b = be.get(key) or {}
        row(f"{label} vs real next opponent %", b.get("opp"), p.get("opp"))
        row(f"{label} win-rate ratio to Era %",
            (100 * b["opp"] / be["era_opp"]) if b.get("opp") and be.get("era_opp") else None,
            (100 * p["opp"] / ce["era_opp"]) if p.get("opp") and ce.get("era_opp") else None)
    s40 = st.get("search40") or {}
    row("with search, 40 games (place)", None, s40.get("place"))
    row("with search, top-4 %", None, 100 * s40["top4"] if s40.get("top4") is not None else None)
    row("with search, cycles a turn", None, s40.get("cycles_per_turn"))
    row("with search, seconds a decision", None, s40.get("sec_per_decision"))
    lines += ["", "The start column is train/runs/xit-0, the same tests on the "
              "checkpoint the loop started from. Search games: "
              f"{tests.get('search', {}).get('games_per_hour', '')} games/hour.", ""]
    (folder / "report.md").write_text("\n".join(lines) + "\n")


def fmt(v) -> str:
    if v is None:
        return ""
    if isinstance(v, float):
        return f"{v:.3f}"
    return str(v)


def args():
    a = argparse.ArgumentParser()
    a.add_argument("--iterations", type=int, default=1,
                   help="iterations to run in this invocation")
    a.add_argument("--start-iteration", type=int, default=1)
    a.add_argument("--init-from", default=str(DEFAULT_INIT),
                   help="the network the loop starts from")
    a.add_argument("--games", type=int, default=1000, help="search games per iteration")
    a.add_argument("--workers", type=int, default=8, help="search_games workers")
    a.add_argument("--batch", type=int, default=1, help="games per search worker")
    a.add_argument("--keep-data", type=int, default=2,
                   help="iterations of search data to train on (current plus this many minus 1)")
    a.add_argument("--seed", type=int, default=11)
    a.add_argument("--eval-games", type=int, default=512, help="vs real and vs standard")
    a.add_argument("--shell-games", type=int, default=200, help="games to collect Shell positions")
    a.add_argument("--shell-positions", type=int, default=40)
    a.add_argument("--cycle-turns", type=int, default=200, help="Era turns in the cycle test")
    a.add_argument("--from-turn", type=int, default=6)
    a.add_argument("--era-sample", type=int, default=150)
    a.add_argument("--era-repeats", type=int, default=3)
    a.add_argument("--search-games", type=int, default=40,
                   help="games of the with-search placement test")
    a.add_argument("--with-search", action=argparse.BooleanOptionalAction, default=True,
                   help="run the with-search placement test")
    a.add_argument("--era-players", default="plain,search",
                   help="players for the Era-turn benchmark")
    a.add_argument("--imitate-lr", type=float, default=0.0,
                   help="imitate.py --lr (0: its default)")
    a.add_argument("--anchor-kl", type=float, default=0.0,
                   help="imitate.py --anchor-kl (0: off)")
    a.add_argument("--dom-weight", type=float, default=1.0,
                   help="imitate.py --dom-weight (1: off)")
    a.add_argument("--imitate-epochs", type=int, default=0,
                   help="imitate.py --max-epochs (0: its default)")
    a.add_argument("--playouts", type=int, default=1)
    a.add_argument("--fights", type=int, default=20)
    a.add_argument("--fight-weight", type=float, default=1.0)
    a.add_argument("--replan", choices=["every", "turn", "cycle"], default="every")
    a.add_argument("--playout-temperature", type=float, default=0.0)
    a.add_argument("--playout-actions", type=int, default=60)
    a.add_argument("--margin", type=float, default=0.0)
    a.add_argument("--real-boards", default=str(ROOT / "boards" / "real"))
    a.add_argument("--baseline", action=argparse.BooleanOptionalAction, default=True,
                   help="measure the starting network in xit-0 first")
    a.add_argument("--resume", action=argparse.BooleanOptionalAction, default=True)
    a.add_argument("--force-measure", action="store_true",
                   help="rerun every measurement stage even when tests.json has it")
    a.add_argument("--vram-gb", type=float, default=0.0,
                   help="cap this process and the search workers at this much GPU memory")
    return a.parse_args()


def main() -> None:
    cfg = args()
    dev = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    if dev.type == "cuda" and cfg.vram_gb > 0:
        total = torch.cuda.get_device_properties(0).total_memory / 2**30
        torch.cuda.set_per_process_memory_fraction(min(1.0, cfg.vram_gb / total), 0)
        log(f"GPU capped at {cfg.vram_gb} GB")
    boards, unknown = real_boards.load(Path(cfg.real_boards))
    log(f"real boards: {len(boards)} ({len(unknown)} unknown minions)")
    runs = ROOT / "train" / "runs"
    runs.mkdir(parents=True, exist_ok=True)

    # The starting point and its baseline.
    if cfg.baseline:
        base = runs / "xit-0"
        base.mkdir(parents=True, exist_ok=True)
        ck = cfg.init_from
        if not (base / "ckpt.pt").exists():
            shutil.copy2(ck, base / "ckpt.pt")
        net = load(str(base / "ckpt.pt"), dev)
        net.eval()
        bsha = sha(base / "ckpt.pt")
        btests = tests_of(base)
        btests["net"] = str(base / "ckpt.pt")
        btests["net_sha"] = bsha
        log(f"baseline {base}: plain network on the starting checkpoint")
        measure_plain(net, dev, boards, cfg, base, btests, bsha,
                      force=cfg.force_measure)
        write_report(base, btests, None, cfg, "Baseline, no search games or training.")
        del net
    else:
        base = None
    baseline = tests_of(runs / "xit-0") if base is not None else None

    current = Path(cfg.init_from)
    for n in range(cfg.start_iteration, cfg.start_iteration + cfg.iterations):
        folder = runs / f"xit-{n}"
        folder.mkdir(parents=True, exist_ok=True)
        if not (folder / "ckpt.pt").exists():
            if n == 1:
                src = Path(cfg.init_from)
            else:
                src = runs / f"xit-{n - 1}" / "ckpt.pt"
            shutil.copy2(src, folder / "ckpt.pt")
        ck = folder / "ckpt.pt"
        net_sha = sha(ck)
        tests = tests_of(folder)
        if tests.get("net_sha") != net_sha:
            tests = {"net": str(ck), "net_sha": net_sha, "stages": {}}
            save_tests(folder, tests)
        tests["net"] = str(ck)
        log(f"\n=== iteration {n}: {ck} ({net_sha[:12]}) ===")

        # (a) search games with the current network.
        data = folder / "search"
        ssum = read_json(data / "search_games.json", {})
        if not (cfg.resume and ssum.get("games", 0) >= cfg.games):
            cmd = [PYTHON, str(ROOT / "train" / "search_games.py"),
                   "--games", str(cfg.games), "--workers", str(cfg.workers),
                   "--batch", str(cfg.batch), "--ckpt", str(ck),
                   "--out", str(data), "--tag", f"xit-{n}", "--seed", str(cfg.seed),
                   "--real-boards", cfg.real_boards,
                   "--playouts", str(cfg.playouts), "--fights", str(cfg.fights),
                   "--fight-weight", str(cfg.fight_weight), "--replan", cfg.replan,
                   "--playout-temperature", str(cfg.playout_temperature),
                   "--playout-actions", str(cfg.playout_actions),
                   "--margin", str(cfg.margin)]
            if cfg.vram_gb > 0:
                cmd += ["--vram-gb", str(cfg.vram_gb)]
            run(cmd, folder / "search_games.log")
            ssum = read_json(data / "search_games.json", {})
        tests["search"] = ssum
        save_tests(folder, tests)
        if not (data / "header.json").exists():
            raise SystemExit(f"{data}: search_games wrote no recording")
        # The merged recording is complete; the per-worker shards were only
        # for resuming a crashed run and take as much disk again.
        if (data / "shards").exists() and (data / "search_games.json").exists():
            shutil.rmtree(data / "shards")
        log(f"  search data: {ssum}")

        # (b) imitation from the current network on up to --keep-data iterations.
        train_data = []
        for k in range(max(1, cfg.keep_data)):
            d = runs / f"xit-{n - k}" / "search"
            if (d / "header.json").exists():
                train_data.append(str(d))
        plain_data = folder / "plain"
        if (plain_data / "header.json").exists():
            train_data.append(str(plain_data))
        best = folder / "imitate" / "best.pt"
        if not (cfg.resume and best.exists()):
            cmd = [PYTHON, str(ROOT / "train" / "imitate.py"),
                   "--data", *train_data, "--out", str(folder / "imitate"),
                   "--init-from", str(ck), "--weight", "none", "--value",
                   "--value-weight", "0.5", "--seed", str(cfg.seed)]
            if cfg.imitate_lr > 0:
                cmd += ["--lr", str(cfg.imitate_lr)]
            if cfg.anchor_kl > 0:
                cmd += ["--anchor-kl", str(cfg.anchor_kl)]
            if cfg.dom_weight != 1.0:
                cmd += ["--dom-weight", str(cfg.dom_weight)]
            if cfg.imitate_epochs > 0:
                cmd += ["--max-epochs", str(cfg.imitate_epochs)]
            run(cmd, folder / "imitate.log")
        if not best.exists():
            raise SystemExit(f"imitate wrote no {best}")
        shutil.copy2(best, ck)
        ck_data = torch.load(best, map_location="cpu")
        tests["imitate"] = {
            "data": train_data,
            "best_epoch": int(ck_data.get("epoch", 0)),
            "val_top1": float(ck_data.get("val_top1", float("nan"))),
            "val_top3": float(ck_data.get("val_top3", float("nan"))),
            "val_loss": float(ck_data.get("val_loss", float("nan"))),
            "val_value_loss": float(ck_data.get("val_value_loss", float("nan"))),
        }
        net_sha = sha(ck)
        tests["net_sha"] = net_sha
        save_tests(folder, tests)
        write_report(folder, tests, baseline, cfg,
                     "Iteration in progress; later measurements are filled in as they finish.")
        log(f"  imitate: {tests['imitate']}")

        # (c) measure the new network, plain first.
        net = load(str(ck), dev)
        net.eval()
        measure_plain(net, dev, boards, cfg, folder, tests, net_sha,
                      force=cfg.force_measure, search40=cfg.with_search, era=True)
        write_report(folder, tests, baseline, cfg)
        del net

        broken, why = stop_broken(tests, baseline)
        if broken:
            note = f"STOPPED EARLY: {why}"
            log(note)
            write_report(folder, tests, baseline, cfg, note)
            summary = {"iteration": n, "stopped": True, "why": why,
                       "tests": tests, "baseline": baseline}
            write_json(folder / "summary.json", summary)
            break
        summary = {"iteration": n, "stopped": False, "tests": tests,
                   "baseline": baseline}
        write_json(folder / "summary.json", summary)
        current = ck

        # The next iteration starts from this network.
        nxt = n + 1
        if nxt < cfg.start_iteration + cfg.iterations:
            nf = runs / f"xit-{nxt}"
            nf.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ck, nf / "ckpt.pt")
    log("\ndone")


if __name__ == "__main__":
    main()
