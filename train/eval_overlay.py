"""The comp overlay against seven real boards (docs/briefs/comp-overlay-1.md).

One learner seat plays r1106-keep against seven real-board seats, or seven
network seats with `--opponents network`. With the overlay on, the Rust rules
force the Shell and enabler/payoff choices and add the logit bonus `B` for
buying or playing a payoff (`B/2` for a filler). With `B = 0` the same loop is
the plain network, and a separate 512-game check is the network-only baseline.

    .venv/bin/python train/eval_overlay.py --games 2048 --bonus 8

Output: one table per B, with the Shell and enabler counts, the active games'
comp share and place, and a per comp table: games active, activation turn,
boards ending as the comp, their share and place. `--out DIR` writes
`report.md` and `results.json`.
"""

from __future__ import annotations

import argparse
import json
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
import torch

import bgpy
import eval_search
import ppo
import real_boards
from model import load

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CKPT = ROOT / "train" / "runs" / "r1106-keep" / "ckpt.pt"
DEFAULT_COMPS = ROOT / "data" / "comps" / "comps.toml"
P = bgpy.PLAYERS
# The overlay flag order is in crates/bgcore/src/overlay.rs.
F_ACTIVE = 4
F_COMP = 6


class Bufs:
    """The numpy arrays bgpy fills in (eval_search.py's)."""

    def __init__(self, n):
        self.obs = np.zeros((n, bgpy.OBS_SIZE), np.float32)
        self.mask = np.zeros((n, bgpy.N_ACTIONS), bool)
        self.seat = np.zeros(n, np.int64)
        self.kind = np.zeros(n, np.int64)
        self.places = np.zeros((n, P), np.int8)
        self.tier_turns = np.zeros((n, P, bgpy.MAX_TIER), np.int8)
        self.tribes = np.zeros((n, P, bgpy.TRIBES), np.int8)
        self.reset = np.zeros(n, bool)
        self.kinds = np.zeros((n, P), np.int64)
        self.forced = np.full((n, P), -1, np.int64)
        self.bonus = np.zeros((n, P, bgpy.N_ACTIONS), np.float32)
        self.flags = np.zeros((n, P, bgpy.OVERLAY_FLAGS), np.int32)


@dataclass
class GameStats:
    shell_legal: set[int] = field(default_factory=set)
    shell_taken: set[int] = field(default_factory=set)
    forced_sell: set[int] = field(default_factory=set)
    enabler_offered: bool = False
    enabler_taken: bool = False
    active_turn: int | None = None
    # Comp index -> the first turn it was active in this game.
    comp_turns: dict[int, int] = field(default_factory=dict)
    place: int = 0
    final: list[int] = field(default_factory=list)


def sample_bonus(net, obs, mask, gen, dev, bonus=None, forced=None):
    """Sample like eval_search.sample, adding the overlay bonus and taking a
    forced action where the overlay gives one."""
    o = torch.from_numpy(obs).to(dev)
    m = torch.from_numpy(mask).to(dev)
    with torch.no_grad():
        logits, _ = net(o)
    m = ppo.force_mask(o, m)
    logits = logits.float().cpu()
    if bonus is not None:
        logits = logits + torch.from_numpy(bonus)
    logits = eval_search.masked(logits, m.cpu())
    probs = torch.softmax(logits, -1)
    actions = torch.multinomial(probs, 1, generator=gen).squeeze(1).numpy()
    if forced is not None:
        take = forced >= 0
        actions[take] = forced[take]
    return actions


def stats_of(b, flags, i, seat, turn):
    """Fold one decision's flags into game i's stats."""
    f = flags[i, seat]
    s = stats_of.games[i]
    if f[0]:
        s.shell_legal.add(turn)
    if f[1]:
        s.shell_taken.add(turn)
    if f[2]:
        s.enabler_offered = True
    if f[3]:
        s.enabler_taken = True
    if f[F_ACTIVE]:
        ci = int(f[F_COMP]) - 1
        if ci >= 0 and ci not in s.comp_turns:
            s.comp_turns[ci] = turn
        if s.active_turn is None:
            s.active_turn = turn
    if f[5]:
        s.forced_sell.add(turn)


stats_of.games = []


def play(net, n_games, seed, dev, boards, comps_path=None, bonus=0.0,
         gen_seed=0, opponents="real"):
    """Play `n_games` with the overlay on (`bonus > 0`) or off.

    Off uses the exact eval_search.play path, so it reproduces the 5.867
    baseline. On uses the same loop with the overlay call. Returns
    (records, game_stats, timing)."""
    t = time.time()
    real = opponents == "real"
    lineup = (1, 0, 0, 0, 0, 7) if real else (1, 7, 0, 0, 0, 0)
    boards_arg = boards if real else None
    if bonus <= 0.0:
        records, _ = eval_search.play(
            net, n_games, seed, dev, lineup=lineup,
            boards=boards_arg, gen_seed=gen_seed, track=False,
        )
        return records, [], {"seconds": time.time() - t, "overlay_ns": 0, "steps": 0}

    n = min(n_games, 512)
    env = bgpy.VecEnv(n, seed, list(lineup), real_boards=boards_arg,
                      old_boards=ppo.OLD_BOARDS,
                      real_lobby_filter=ppo.REAL_LOBBY_FILTER)
    env.set_overlay(str(comps_path), float(bonus))
    b = Bufs(n)
    env.kinds(b.kinds)
    gen = torch.Generator(device="cpu")
    gen.manual_seed(int(gen_seed) & 0xFFFF_FFFF)
    records, done = [], 0
    turns = np.zeros(n, np.int64)
    game_stats = [GameStats() for _ in range(n)]
    completed: list[GameStats] = []
    stats_of.games = game_stats
    overlay_ns = 0
    steps = 0
    while done < n_games:
        env.observe(b.obs, b.mask, b.seat, b.kind)
        env.remember_boards()
        t0 = time.perf_counter_ns()
        env.overlay(b.forced, b.bonus, b.flags)
        overlay_ns += time.perf_counter_ns() - t0
        steps += 1
        for i in range(n):
            if b.kind[i] != bgpy.LEARNER:
                continue
            seat = int(b.seat[i])
            turn = int(b.obs[i, bgpy.SCALAR_TURN])
            stats_of(b, b.flags, i, seat, turn)
        actions = sample_bonus(
            net, b.obs, b.mask, gen, dev,
            bonus=b.bonus[np.arange(n), b.seat],
            forced=b.forced[np.arange(n), b.seat],
        )
        turn = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
        turns += (turn > np.maximum(turns, 0)) & (b.kind == bgpy.LEARNER)
        kinds = b.kinds.copy()
        env.step(actions, b.places, b.tier_turns, b.tribes, b.reset)
        # A seat's placement is reported on the step it finishes, which is not
        # always the step the whole game ends on: keep the first one seen.
        for i in range(n):
            for s in np.nonzero(kinds[i] == bgpy.LEARNER)[0]:
                if b.places[i, s]:
                    game_stats[i].place = int(b.places[i, s])
        if b.reset.any():
            finals = env.final_boards()
            for i in np.nonzero(b.reset)[0]:
                seats = np.nonzero(kinds[i] == bgpy.LEARNER)[0]
                if len(seats) != 1:
                    raise SystemExit("the overlay eval expects one learner seat")
                s = int(seats[0])
                st = game_stats[i]
                if not st.place and b.places[i, s]:
                    st.place = int(b.places[i, s])
                st.final = [int(x) for x in finals[i][s] if x >= 0]
                records.append(
                    (
                        st.place,
                        b.tier_turns[i, s].astype(int).tolist(),
                        b.tribes[i, s].astype(int).tolist(),
                        int(turns[i]),
                    )
                )
                completed.append(st)
                game_stats[i] = GameStats()
                stats_of.games[i] = game_stats[i]
                turns[i] = 0
            env.kinds(b.kinds)
        done += int(b.reset.sum())
    return records, completed, {"seconds": time.time() - t, "overlay_ns": overlay_ns, "steps": steps}


def ends_as_comp(final, spec):
    """A final board that is the comp: one enabler entry complete and at
    least two of its core cards."""
    final_set = set(final)
    if not spec["enabler_groups_ids"]:
        return False
    complete = any(
        all(any(card in final_set for card in group) for group in entry)
        for entry in spec["enabler_groups_ids"]
    )
    return complete and sum(1 for card in spec["core_ids"] if card in final_set) >= 2


def summarize(records, game_stats, comps):
    places = np.array([r[0] for r in records], float)
    n = len(places)
    rng = np.random.default_rng(7)
    boots = [places[rng.integers(0, n, n)].mean() for _ in range(2000)]
    active = [s for s in game_stats if s.active_turn is not None]
    any_comp_board = [
        s for s in active
        if any(ends_as_comp(s.final, spec) for spec in comps)
    ]
    per_comp = []
    for i, spec in enumerate(comps):
        games = [s for s in game_stats if i in s.comp_turns]
        ending = [s for s in games if ends_as_comp(s.final, spec)]
        per_comp.append({
            "index": i,
            "name": spec["name"],
            "active": len(games),
            "active_turn": round(float(np.mean([s.comp_turns[i] for s in games])), 2) if games else 0.0,
            "active_place": round(float(np.mean([s.place for s in games])), 3) if games else 0.0,
            "comp_boards": len(ending),
            "comp_share": round(len(ending) / len(games), 3) if games else 0.0,
            "comp_place": round(float(np.mean([s.place for s in ending])), 3) if ending else 0.0,
        })
    return {
        "games": n,
        "place": round(float(places.mean()), 3),
        "place_ci": (round(float(np.percentile(boots, 2.5)), 3),
                     round(float(np.percentile(boots, 97.5)), 3)),
        "top4": round(float((places <= 4).mean()), 3),
        "first": round(float((places == 1).mean()), 3),
        "shell_legal": sum(len(s.shell_legal) for s in game_stats),
        "shell_taken": sum(len(s.shell_taken) for s in game_stats),
        "forced_sell": sum(len(s.forced_sell) for s in game_stats),
        "enabler_offered": sum(1 for s in game_stats if s.enabler_offered),
        "enabler_taken": sum(1 for s in game_stats if s.enabler_taken),
        "active": len(active),
        "active_turn": round(float(np.mean([s.active_turn for s in active])), 2) if active else 0.0,
        "active_place": round(float(np.mean([s.place for s in active])), 3) if active else 0.0,
        "active_share": round(len(active) / n, 3) if n else 0.0,
        "comp_boards": len(any_comp_board),
        "comp_share": round(len(any_comp_board) / len(active), 3) if active else 0.0,
        "per_comp": per_comp,
    }


def load_comps(path):
    """Every comp's enabler groups, core and payoff/filler names."""
    data = tomllib.loads(Path(path).read_text())
    out = []
    for c in data.get("comp", []):
        groups = [list(e.get("all", [])) for e in c.get("enabler", [])]
        names = sorted({name for entry in groups for group in entry for name in group})
        out.append({
            "name": c.get("name", "?"),
            "enabler_groups": groups,
            "enabler_names": names,
            "core": [core["name"] for core in c.get("core", [])],
            "payoff": c.get("payoff", []),
            "filler": c.get("filler", []),
        })
    return out


def names_to_ids(names, wanted):
    out = []
    for name in wanted:
        if name not in names:
            raise SystemExit(f"card {name!r} is not in the card set")
        out.append(names.index(name))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[1])
    ap.add_argument("--ckpt", default=str(DEFAULT_CKPT))
    ap.add_argument("--comps", default=str(DEFAULT_COMPS))
    ap.add_argument("--games", type=int, default=2048)
    ap.add_argument("--bonus", type=float, nargs="*", default=[8],
                    help="B values; 0 is the overlay off")
    ap.add_argument("--seed", type=int, default=14606)
    ap.add_argument("--opponents", choices=("real", "network"), default="real",
                    help="seven real-board seats (filter ON) or seven network seats")
    ap.add_argument("--real-boards", default=str(ROOT / "boards" / "real"))
    ap.add_argument("--out", default=None)
    ap.add_argument("--vram-gb", type=float, default=6.0,
                    help="cap this process's GPU memory (6 while Era games)")
    ap.add_argument("--real-lobby-filter", action="store_true",
                    help="a real seat draws only boards whose minions fit this game's lobby")
    args = ap.parse_args()

    dev = "cuda" if torch.cuda.is_available() else "cpu"
    if dev == "cuda" and args.vram_gb > 0:
        torch.cuda.set_per_process_memory_fraction(args.vram_gb / 16.3)
        print(f"GPU capped at {args.vram_gb} GB")
    ppo.FORCE_DARK = True
    ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = "keep"
    ppo.REAL_LOBBY_FILTER = args.real_lobby_filter
    net = load(args.ckpt, dev)
    net.eval()
    specs, unknown = real_boards.load(Path(args.real_boards))
    print(f"{args.ckpt}: {dev}; {len(specs)} real boards ({len(unknown)} unknown minions); "
          f"opponents {args.opponents}, filter {args.real_lobby_filter}")

    comp_specs = load_comps(args.comps)
    names = None
    results = {}
    for b in args.bonus:
        t = time.time()
        records, game_stats, timing = play(
            net, args.games, args.seed, dev, specs, args.comps, b,
            gen_seed=args.seed, opponents=args.opponents,
        )
        if names is None:
            names = net_card_names()
        comps = [
            {
                **spec,
                "core_ids": names_to_ids(names, spec["core"]),
                "enabler_groups_ids": [
                    [names_to_ids(names, group) for group in entry]
                    for entry in spec["enabler_groups"]
                ],
                "payoff_ids": names_to_ids(names, spec["payoff"]),
                "filler_ids": names_to_ids(names, spec["filler"]),
            }
            for spec in comp_specs
        ]
        summary = summarize(records, game_stats, comps)
        summary["seconds"] = round(time.time() - t, 2)
        summary["overlay_ms_per_step"] = (
            round(timing["overlay_ns"] / timing["steps"] / 1e6, 4) if timing["steps"] else 0.0
        )
        summary["overlay_share"] = (
            round(100.0 * timing["overlay_ns"] / (timing["seconds"] * 1e9), 2) if timing["steps"] else 0.0
        )
        summary["label"] = f"B {b:g}" if b > 0 else "off"
        results[b] = summary
        if b == 0:
            print(
                f"off: place {summary['place']} {summary['place_ci']}, "
                f"top4 {summary['top4']:.1%}, first {summary['first']:.1%}, "
                f"{summary['seconds']} s (rules not recorded)",
                flush=True,
            )
        else:
            print(
                f"{summary['label']}: place {summary['place']} {summary['place_ci']}, "
                f"top4 {summary['top4']:.1%}, first {summary['first']:.1%}, "
                f"shell {summary['shell_taken']}/{summary['shell_legal']} "
                f"({0 if not summary['shell_legal'] else 100*summary['shell_taken']/summary['shell_legal']:.1f}%), "
                f"enabler offered/taken {summary['enabler_offered']}/{summary['enabler_taken']}, "
                f"active {summary['active']} ({summary['active_share']:.1%}, turn {summary['active_turn']}), "
                f"comp {summary['comp_share']:.1%} of active, active place {summary['active_place']}, "
                f"forced sells {summary['forced_sell']}, {summary['seconds']} s",
                flush=True,
            )

    # The network-only baseline, same seeds and opponents.
    print("\n512-game overlay-off check:")
    rec, _ = eval_search.play(
        net, 512, args.seed, dev,
        lineup=(1, 0, 0, 0, 0, 7) if args.opponents == "real" else (1, 7, 0, 0, 0, 0),
        boards=specs if args.opponents == "real" else None,
        gen_seed=args.seed, track=False,
    )
    s = eval_search.summarize(rec)
    print(f"  off 512: place {s['place']} {s['place_ci']}, top4 {s['top4']:.1%}, first {s['first']:.1%}")

    print("\n| B | place (95%) | top4 | first | Shell taken/legal | enabler offered/taken | active | active turn | comp boards | active place | forced sells |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for b in args.bonus:
        r = results[b]
        if b == 0:
            print(
                f"| {r['label']} | {r['place']} ({r['place_ci'][0]} to {r['place_ci'][1]}) | "
                f"{r['top4']:.1%} | {r['first']:.1%} | - | - | - | - | - | - | - |"
            )
            continue
        rate = 0 if not r["shell_legal"] else 100 * r["shell_taken"] / r["shell_legal"]
        print(
            f"| {r['label']} | {r['place']} ({r['place_ci'][0]} to {r['place_ci'][1]}) | "
            f"{r['top4']:.1%} | {r['first']:.1%} | {r['shell_taken']}/{r['shell_legal']} ({rate:.1f}%) | "
            f"{r['enabler_offered']}/{r['enabler_taken']} | {r['active']} ({r['active_share']:.1%}) | "
            f"{r['active_turn']} | {r['comp_boards']} ({r['comp_share']:.1%}) | {r['active_place']} | {r['forced_sell']} |"
        )

    print("\nPer comp (B > 0):")
    print("| comp | games active | activation turn | ending as comp | share | active place | comp place |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    for b in args.bonus:
        if b <= 0:
            continue
        for c in results[b]["per_comp"]:
            print(
                f"| {c['name']} | {c['active']} | {c['active_turn']} | {c['comp_boards']} | "
                f"{c['comp_share']:.1%} | {c['active_place']} | {c['comp_place']} |"
            )
    for b in args.bonus:
        if b <= 0:
            continue
        r = results[b]
        print(f"\nB {b:g}: any comp active in {r['active']}/{r['games']} = {r['active_share']:.1%} "
              f"(target at least a third: {'PASS' if r['active_share'] >= 1/3 else 'FAIL'})")
        print("Overlay call cost per step (B > 0):")
        print(f"  B {b:g}: {r['overlay_ms_per_step']} ms/step, {r['overlay_share']}% of the loop")

    if args.out:
        out = Path(args.out)
        out.mkdir(parents=True, exist_ok=True)
        clean = {str(k): v for k, v in results.items()}
        (out / "results.json").write_text(json.dumps(clean, indent=2) + "\n")
        lines = ["# Comp overlay eval", ""]
        for b in args.bonus:
            r = results[b]
            lines.append(f"- {r['label']}: place {r['place']} {r['place_ci']}, top4 {r['top4']:.1%}, "
                         f"active {r['active']} ({r['active_share']:.1%}), comp share {r['comp_share']:.1%}")
            for c in r["per_comp"]:
                lines.append(f"  - {c['name']}: active {c['active']}, turn {c['active_turn']}, "
                             f"comp {c['comp_boards']} ({c['comp_share']:.1%}), place {c['active_place']}")
        (out / "report.md").write_text("\n".join(lines) + "\n")


def net_card_names():
    """The card names by id, from the bundled data (no env needed)."""
    return bgpy.VecEnv(1, 0, [8, 0, 0, 0, 0, 0], real_boards=None).card_names()


if __name__ == "__main__":
    main()
