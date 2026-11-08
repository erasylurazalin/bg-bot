"""Play test games with a checkpoint and show the boards the learner ends on.

    .venv/bin/python train/show_boards.py runs/r1102-deadline/best.pt
    .venv/bin/python train/show_boards.py CKPT --vs std --games 64

One learner seat against 7 real boards (default) or 7 standard or rush bots.
For each game it prints the learner's last board (taken after its last
action before it was knocked out or the game ended), its hero, tier and
placement, then a summary: placement by tribe, and the minions most common
on top-4 and bottom-4 boards. Writes the full text to runs/boards-<name>.txt.
"""

import argparse
from collections import Counter
from pathlib import Path

import numpy as np
import torch

import bgpy
import ppo
import real_boards
from model import load
from ppo import LEARNER, Bufs, act, force_mask

LINEUPS = {"real": [1, 0, 0, 0, 0, 7], "std": [1, 0, 7, 0, 0, 0], "rush": [1, 0, 0, 7, 0, 0]}
TRIBE_NAMES = ("beast", "mech", "dragon", "murloc", "demon", "pirate",
               "elemental", "undead", "quilboar", "aberration")


def board_tribe(tribes):
    """The board's tribe as ppo.py counts it: the most common, at least 4."""
    tribes = tribes[:len(TRIBE_NAMES)]
    k = int(np.argmax(tribes))
    return TRIBE_NAMES[k] if tribes[k] >= 4 else "mixed"


def fmt_minion(m):
    name, golden, atk, hp, kw = m
    s = f"{'golden ' if golden else ''}{name} {atk}/{hp}"
    return s + (f" [{kw}]" if kw else "")


@torch.no_grad()
def main():
    a = argparse.ArgumentParser()
    a.add_argument("ckpt")
    a.add_argument("--vs", choices=LINEUPS, default="real")
    a.add_argument("--games", type=int, default=48)
    a.add_argument("--seed", type=int, default=777)
    a.add_argument("--force-dark-gift", action="store_true",
                   help="press Dark Discovery on turns 3 to 5 (on by default when the run trained with it)")
    a.add_argument("--cpu", action="store_true", help="run on the CPU (while training holds the GPU)")
    cfg = a.parse_args()

    dev = "cuda" if torch.cuda.is_available() and not cfg.cpu else "cpu"
    net = load(cfg.ckpt, dev).eval()
    trained = torch.load(cfg.ckpt, map_location="cpu", weights_only=False).get("cfg", {})
    ppo.FORCE_DARK = cfg.force_dark_gift or bool(trained.get("force_dark_gift"))
    real = None
    if cfg.vs == "real":
        real, _ = real_boards.load(real_boards.DEFAULT_DIR)
    n = min(cfg.games, 32)
    env = bgpy.VecEnv(n, cfg.seed, LINEUPS[cfg.vs], real_boards=real)
    b = Bufs(n)
    env.kinds(b.kinds)
    results = []
    while len(results) < cfg.games:
        before = [env.seats(i) for i in range(n)]
        env.observe(b.obs, b.mask, b.seat, b.kind)
        turns = b.obs[:, bgpy.SCALAR_TURN].astype(int)
        o = torch.from_numpy(b.obs).to(dev)
        acts, _, _, _ = act(net, o, force_mask(o, torch.from_numpy(b.mask).to(dev)))
        kinds = b.kinds.copy()
        env.step(acts.cpu().numpy(), b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kinds[e, s] != LEARNER:
                continue
            _, _, _, tier, hero, board = before[e][s]
            tt = [int(t) for t in b.tier_turns[e, s][1:5]]
            results.append({
                "place": int(b.places[e, s]), "tier": tier, "hero": hero, "board": board,
                "tribe": board_tribe(b.tribes[e, s]), "tier_turns": tt,
                "turn": int(turns[e]),
            })
        if b.reset.any():
            env.kinds(b.kinds)
    results = results[:cfg.games]

    out = []
    for r in sorted(results, key=lambda r: r["place"]):
        tt = "/".join(str(t) if t else "never" for t in r["tier_turns"])
        out.append(f"#{r['place']}  turn {r['turn']}  {r['hero'] or 'no hero'}  tier {r['tier']}  "
                   f"({r['tribe']}; reached tiers 2/3/4/5 on turns {tt})")
        out.append("    " + (", ".join(fmt_minion(m) for m in r["board"]) or "(empty board)"))
    out.append("")
    places = [r["place"] for r in results]
    out.append(f"{len(results)} games vs 7 {cfg.vs}: "
               f"avg place {np.mean(places):.2f}, top 4 {np.mean([p <= 4 for p in places]):.0%}, "
               f"first {np.mean([p == 1 for p in places]):.0%}")
    out.append("knocked out on turn: " + ", ".join(
        f"t{t} {c}" for t, c in sorted(Counter(r["turn"] for r in results if r["place"] > 1).items())))
    by_tribe = Counter(r["tribe"] for r in results)
    out.append("by tribe: " + ", ".join(
        f"{t} {c} (avg {np.mean([r['place'] for r in results if r['tribe'] == t]):.2f})"
        for t, c in by_tribe.most_common()))
    for label, sel in (("top 4", lambda p: p <= 4), ("bottom 4", lambda p: p > 4)):
        rs = [r for r in results if sel(r["place"])]
        cnt = Counter(m[0] for r in rs for m in r["board"])
        out.append(f"most common on {label} boards ({len(rs)}): " + ", ".join(
            f"{name} {c}" for name, c in cnt.most_common(15)))
    text = "\n".join(out)
    print(text)
    name = Path(cfg.ckpt).parent.name + f"-{cfg.vs}"
    path = Path(__file__).parent / "runs" / f"boards-{name}.txt"
    path.write_text(text + "\n")
    print(f"\nwritten to {path}")


if __name__ == "__main__":
    main()
