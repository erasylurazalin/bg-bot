"""The search's evaluation: N games with one learner seat and seven real-board
seats, played once with the plain network and once with the search over the
same seeds, plus the search brief's four tests (docs/briefs/).

    .venv/bin/python train/eval_search.py --games 200
    .venv/bin/python train/eval_search.py --shell-test
    .venv/bin/python train/eval_search.py --warden-test
    .venv/bin/python train/eval_search.py --cycle-rate --from-turn 6

The placement table is the plain network against the search at
`--fight-weight` 0, 0.5, 1 and 2. Every table reports average place (with a
bootstrap interval over games), top-4 and first rate, the turns the learner
reached tiers 3, 4 and 5, how many final boards were one tribe against mixed,
the cards it bought most, the cycles it played per turn, how often the
dominance judge fired, and the seconds per decision.
"""

from __future__ import annotations

import argparse
import time
from pathlib import Path

import numpy as np
import torch

import bgpy
import ppo
import real_boards
from model import load, masked
from search import K_BUY, KIND_OF, KINDS, Searcher, Slots, TurnPlay

ROOT = Path(__file__).resolve().parent.parent
LEARNER = bgpy.LEARNER
P = bgpy.PLAYERS
DEFAULT_CKPT = ROOT / "train" / "runs" / "r1106-keep" / "ckpt.pt"
SHELL = "Tortollan Blue Shell"
TRIBE_NAMES = (
    "beast", "mech", "dragon", "murloc", "demon", "pirate",
    "elemental", "undead", "quilboar", "aberration",
)


# ---- the games -----------------------------------------------------------


class Bufs:
    """The numpy arrays bgpy fills in, one set per VecEnv (ppo.py's)."""

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


def forced(o, m):
    """ppo.py's own forced mask: the eval's Dark Gift presses (turns 3 to 5)
    and the turn 2 level, when the run trained with them. `o` and `m` are
    tensors on the eval device, as in `ppo._evaluate`."""
    return ppo.force_mask(o, m)


def sample(net, obs, mask, gen, dev, force=True):
    """The network's action for each row, sampled at temperature 1, with the
    same forced mask ppo.py's `_evaluate` uses (so this is the same player)."""
    o = torch.from_numpy(obs).to(dev)
    m = torch.from_numpy(mask).to(dev)
    with torch.no_grad():
        logits, _ = net(o)
    if force:
        m = forced(o, m)
    logits = masked(logits.float().cpu(), m.cpu())
    probs = torch.softmax(logits, -1)
    return torch.multinomial(probs, 1, generator=gen).squeeze(1).numpy()


def play(net, n_games, seed, dev, lineup=(1, 0, 0, 0, 0, 7), boards=None,
         searcher=None, gen_seed=0, track=True, max_rows=512):
    """Play `n_games` with `lineup`, one decision at a time (`searcher` for the
    learner seats, else the plain network). Returns the learner records."""
    n = min(n_games, max_rows)
    env = bgpy.VecEnv(n, seed, list(lineup), real_boards=boards,
                      old_boards=ppo.OLD_BOARDS)
    b = Bufs(n)
    env.kinds(b.kinds)
    gen = torch.Generator(device="cpu")
    gen.manual_seed(int(gen_seed) & 0xFFFF_FFFF)
    names = env.card_names()
    if searcher is not None:
        searcher.names = names
        searcher.force = forced
    records, done = [], 0
    turns = np.zeros(n, np.int64)
    buys: dict[str, int] = {}
    while done < n_games:
        env.observe(b.obs, b.mask, b.seat, b.kind)
        if searcher is not None:
            actions = np.array(
                [searcher.decide(env, i, int(b.seat[i])) for i in range(n)], np.int64
            )
        else:
            actions = sample(net, b.obs, b.mask, gen, dev)
        if track:
            for i in range(n):
                a = int(actions[i])
                if KIND_OF[a] != K_BUY:
                    continue
                d = env.search_copy(i).describe(0)
                if a < len(d[8]):
                    name = names[d[8][a][0]]
                    buys[name] = buys.get(name, 0) + 1
        turn = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
        turns += (turn > np.maximum(turns, 0)) & (b.kind == LEARNER)
        kinds = b.kinds.copy()
        env.step(actions, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kinds[e, s] == LEARNER:
                records.append(
                    (
                        int(b.places[e, s]),
                        b.tier_turns[e, s].astype(int).tolist(),
                        b.tribes[e, s].astype(int).tolist(),
                        int(turns[e]),
                    )
                )
        done += int(b.reset.sum())
        if b.reset.any():
            env.kinds(b.kinds)
            if searcher is not None:
                for i in np.nonzero(b.reset)[0]:
                    searcher.plans.clear()
                    searcher._judged.clear()
            turns[b.reset] = 0
    return records, buys


def summarize(records, buys=None, searcher=None) -> dict:
    """The numbers a run reports."""
    if not records:
        return {}
    places = np.array([r[0] for r in records], float)
    tiers = np.array([r[1] for r in records])
    tribes = np.array([r[2] for r in records])
    turns = np.array([r[3] for r in records], float)
    rng = np.random.default_rng(7)
    boots = [places[rng.integers(0, len(places), len(places))].mean() for _ in range(2000)]
    out = {
        "games": len(records),
        "place": round(float(places.mean()), 3),
        "place_ci": (round(float(np.percentile(boots, 2.5)), 3),
                     round(float(np.percentile(boots, 97.5)), 3)),
        "top4": round(float((places <= 4).mean()), 3),
        "first": round(float((places == 1).mean()), 3),
        "turns": round(float(turns.mean()), 2),
    }
    for tier in (3, 4, 5):
        col = tiers[:, tier - 1]
        out[f"turn_t{tier}"] = round(float(col[col > 0].mean()), 2) if (col > 0).any() else 0.0
    # Final boards: one tribe against mixed (the top count decides, a tie is
    # mixed, the same rule as the leaderboard).
    mixed = 0
    mono = {}
    for row in tribes[:, : len(TRIBE_NAMES)]:
        top = row.max()
        if top == 0:
            continue
        winners = [TRIBE_NAMES[i] for i, v in enumerate(row) if v == top]
        if len(winners) > 1:
            mixed += 1
        else:
            mono[winners[0]] = mono.get(winners[0], 0) + 1
    total = mixed + sum(mono.values())
    out["boards"] = total
    out["mixed_share"] = round(mixed / total, 3) if total else 0.0
    out["tribes"] = dict(sorted(mono.items(), key=lambda kv: -kv[1]))
    if buys:
        out["top_buys"] = sorted(buys.items(), key=lambda kv: -kv[1])[:8]
    if searcher is not None:
        st = searcher.stats()
        out["decisions"] = st["decisions"]
        out["decisions_per_game"] = round(st["decisions"] / max(1, len(records)), 1)
        out["dominance_rate"] = round(st["dominance_fired"] / max(1, st["decisions"]), 4)
        out["cycles_per_turn"] = round(st["cycles_played"] / max(1.0, turns.sum()), 3)
        out["sec_per_decision"] = round(st["seconds_per_decision"], 4)
        out["candidates"] = st["candidates_scored"]
    return out


def table(rows: list[tuple[str, dict]]) -> str:
    """The report table for one or more runs."""
    keys = ["games", "place", "place_ci", "top4", "first", "turns", "turn_t3",
            "turn_t4", "turn_t5", "mixed_share", "decisions"]
    head = f"{'run':<22}" + "".join(f"{k:>12}" for k in keys)
    lines = [head, "-" * len(head)]
    for name, s in rows:
        cells = []
        for k in keys:
            v = s.get(k, "")
            if k == "place_ci" and v:
                v = f"{v[0]:.2f}-{v[1]:.2f}"
            cells.append(f"{v:>12}")
        lines.append(f"{name:<22}" + "".join(cells))
    for name, s in rows:
        if "cycles_per_turn" in s:
            lines.append(
                f"{name}: {s['dominance_rate'] * 100:.1f}% of decisions found a dominating "
                f"cycle, {s['cycles_per_turn']:.2f} cycles a turn, "
                f"{s['sec_per_decision']:.3f} s a decision, {s['candidates']} candidates scored"
            )
        if s.get("top_buys"):
            lines.append(f"{name} buys: " + ", ".join(f"{c} {k}" for c, k in s["top_buys"]))
        if s.get("tribes"):
            lines.append(f"{name} final boards: " + ", ".join(f"{t} {k}" for t, k in s["tribes"].items()))
    return "\n".join(lines)


# ---- the tests -----------------------------------------------------------


def shell_positions(net, n_games, seed, dev, boards, limit=40, max_rows=256):
    """Positions where the bot lost its last fight, has 3+ Gold, the Tortollan
    Blue Shell is in the Tavern and there is room in the hand: collected from
    plain-network games.

    A cycle also needs a free board slot (a card is sold from the board, and
    the Shell has to be played to be sold), so the positions are split: the
    ones with room are the ones the search can play, and how many had none is
    reported, because a bot that keeps seven minions can never cycle at all
    while Era keeps a slot for it."""
    env = bgpy.VecEnv(min(n_games, max_rows), seed, [1, 0, 0, 0, 0, 7], real_boards=boards,
                      old_boards=ppo.OLD_BOARDS)
    b = Bufs(env.n)
    gen = torch.Generator(device="cpu")
    gen.manual_seed(int(seed) & 0xFFFF_FFFF)
    names = env.card_names()
    shell = names.index(SHELL)
    found, full_board, done = [], 0, 0
    while done < n_games and len(found) < limit:
        env.observe(b.obs, b.mask, b.seat, b.kind)
        lost = b.obs[:, bgpy.SCALAR_LOST_LAST_COMBAT] > 0
        gold = b.obs[:, bgpy.SCALAR_GOLD]
        for i in np.nonzero(lost & (gold >= 3))[0]:
            batch = env.search_copy(int(i))
            d = batch.describe(0)
            waiting = Slots(batch)
            waiting.observe()
            if waiting.state[0] != 0:
                # A target or a Discover is open: the shop decision the Shell
                # is bought in comes after it, not here.
                continue
            if any(m[0] == shell for m in d[8]) and len(d[7]) < bgpy.HAND:
                if len(d[9]) >= bgpy.BOARD:
                    full_board += 1
                else:
                    found.append(batch)
        actions = sample(net, b.obs, b.mask, gen, dev)
        env.step(actions, b.places, b.tier_turns, b.tribes, b.reset)
        done += int(b.reset.sum())
    return found, names, full_board


def shell_test(net, args, dev, boards) -> None:
    found, names, full_board = shell_positions(
        net, args.games, args.seed, dev, boards, limit=args.positions)
    print(f"collected {len(found)} Blue Shell positions with room to play it, and "
          f"{full_board} where the brief's conditions held but the board was full "
          f"(a cycle cannot be played there at all)")
    result = {"positions": len(found), "full_board": full_board}
    if not found:
        return result
    shell = names.index(SHELL)
    if args.verbose:
        detail(net, args, dev, found[0], names, shell)
    rows = []
    for label, mode in (("plain", "plain"), ("search", "search")):
        bought = sold = dom = 0
        for batch in found:
            copy = batch.copy()
            s = Searcher(net, dev=dev, mode=mode, playouts=args.playouts, fights=args.fights,
                         fight_weight=args.fight_weight, seed=args.seed, replan=args.replan,
                         playout_temperature=args.playout_temperature,
                         playout_actions=args.playout_actions, margin=args.margin)
            s.names = names
            s.force = forced
            s.fights = 0 if not batch.n_fights else s.fights
            tp = TurnPlay(copy, s, names).run()
            did_buy = shell in tp.bought
            did_sell = shell in tp.sold
            bought += did_buy
            sold += did_buy and did_sell
            dom += s.dominance_fired > 0
        n = len(found)
        rows.append((label, {
            "games": n,
            "place": round(100.0 * bought / n, 1),
            "place_ci": (0, 0),
            "top4": round(100.0 * sold / n, 1),
            "first": round(100.0 * dom / n, 1),
        }))
        result[f"{label}_buy"] = bought / n
        result[f"{label}_sell"] = sold / n
        result[f"{label}_dominance"] = dom / n
    print(f"over every one of the {n + full_board} positions the brief names, the "
          f"search's buy-and-sell rate is still counted on the {n} it can play")
    print("columns: place = bought the Shell %, top4 = bought and sold it %, "
          "first = a dominance cycle fired %")
    print(table(rows))
    return result


def detail(net, args, dev, batch, names, shell) -> None:
    """One Blue Shell position in full: the candidates that involve the Shell,
    their cost and the dominance verdict with what blocked it."""
    s = Searcher(net, dev=dev, mode="search", playouts=args.playouts, fights=args.fights,
                 fight_weight=args.fight_weight, seed=args.seed, replan="every",
                 playout_temperature=args.playout_temperature,
                 playout_actions=args.playout_actions, margin=args.margin)
    s.names = names
    s.force = forced
    copy = batch.copy()
    S = Slots(copy)
    S.observe()
    d = S.describe(0)
    print(f"--- Blue Shell position (turn {copy.slot_turn(0)}): gold {d[0]}, tier {d[1]}, "
          f"hand {[names[m[0]] for m in d[7]]}")
    print(f"    shop {[names[m[0]] for m in d[8]]}")
    print(f"    board {[(names[m[0]], m[1], m[2]) for m in d[9]]}")
    print(f"    state {S.state[0]} (0 = a shop decision), {int(S.mask[0].sum())} legal actions")
    cycles = s.cycles(S)
    shell_cycles = [c for c in cycles if shell == _card_of(c, d, names, shell)]
    print(f"    {len(cycles)} cycles, {len(shell_cycles)} of them the Shell:")
    for c in shell_cycles:
        dom, better, blocked = S.dominance(0, c.slot)
        print(f"      {c.variant:<16} cost {c.cost:>3} dominates {dom} better {better} "
              f"actions {len(c.actions)} blocked {blocked[:4]}")
    tp = TurnPlay(batch.copy(), s, names).run()
    print(f"    search plays it: bought {names[shell] in [names[c] for c in tp.bought]}, "
          f"sold {names[shell] in [names[c] for c in tp.sold]}, dominance fired "
          f"{s.dominance_fired}, actions {len(tp.actions)}")


def _card_of(candidate, desc, names, shell: int) -> int:
    """The Shell's card id when a candidate is the Shell's cycle, else -1."""
    return shell if "Tortollan" in candidate.label else -1


def warden_test(net, args, dev) -> None:
    """Era's free Dragon cycle: Brann and Kalecgos with four Dragons, one free
    board slot, a Draconic Warden in the Tavern, 3+ Gold."""
    board = ["Brann Bronzebeard", "Kalecgos, Arcane Aspect", "Bronze Warden",
             "Blue Whelp", "Tarecgosa", "Persistent Poet"]
    batch = bgpy.sandbox_position(args.seed, board, ["Draconic Warden"], [], 10, 5)
    names = batch.card_names()
    s = Searcher(net, dev=dev, mode="search", playouts=args.playouts, fights=0,
                 seed=args.seed, replan="every", log=print,
                 playout_temperature=args.playout_temperature,
                 playout_actions=args.playout_actions, margin=args.margin)
    s.names = names
    S = Slots(batch)
    S.observe()
    before = S.describe(0)
    cycles = s.cycles(S)
    print("cycles found:", len(cycles))
    for c in cycles:
        if "Warden" not in c.label:
            continue
        print(f"  {c.variant:<16} cost {c.cost:>2} dominates {c.dominates} better {c.better}")
        if not c.dominates and c.variant in ("played_first", "generated_first"):
            dom, better, blocked = S.dominance(0, c.slot)
            print("     blocked by:", blocked)
    after = S.describe(0)
    print(f"engine: hand before {[names[m[0]] for m in before[7]]}, "
          f"after {[names[m[0]] for m in S.describe(cycles[0].slot)[7]]}")
    print("board before:", [(names[m[0]], m[1], m[2]) for m in before[9]])
    print("board after: ", [(names[m[0]], m[1], m[2]) for m in S.describe(cycles[0].slot)[9]])
    a = s.decide_batch(batch, key=("warden", 0))
    print("first action:", a, KINDS[KIND_OF[a]], "plan:", len(s.plans.get(("warden", 0), (0, []))[1]),
          "actions")
    print("dominance fired:", s.dominance_fired, "cycles:", s.cycles_played)
    print("gold after the turn:", S.describe(0)[0], "was", before[0])
    del after


def cycle_rate(net, args, dev) -> None:
    """Every replayed Era turn from turn 6 on: how often Era makes a cycle, and
    how often the plain network and the search do from the same start."""
    batches, labels = bgpy.shop_replay_turns(str(ROOT / "boards" / "actions"),
                                             from_turn=args.from_turn, seed=args.seed,
                                             ref_cards=str(ROOT / "ref" / "bg-cards.json"))
    skipped = [l[0] for l in labels if l[1] == 0]
    labels = [l for l in labels if l[1] != 0]
    print(f"{len(batches)} Era turns from turn {args.from_turn} on "
          f"({len({l[0] for l in labels})} games)"
          + (f"; {len(skipped)} action file(s) do not load: {skipped[:3]}" if skipped else ""))
    rows = []
    era = 0
    # Era's own cycles: from the logged steps of each turn (a card bought or
    # played and sold in the same turn, by the action files).
    era = era_cycles(ROOT / "boards" / "actions", args.from_turn)
    for mode in ("plain", "search"):
        with_cycle = 0
        total_cycles = 0
        for batch in batches[: args.turns]:
            s = Searcher(net, dev=dev, mode=mode, playouts=args.playouts, fights=0,
                         fight_weight=0.0, seed=args.seed, replan=args.replan,
                         playout_temperature=args.playout_temperature,
                         playout_actions=args.playout_actions, margin=args.margin)
            turn = batch.copy()
            s.names = turn.card_names()
            tp = TurnPlay(turn, s, s.names).run()
            cyc = tp.cycles()
            with_cycle += bool(cyc)
            total_cycles += len(cyc)
        n = len(batches[: args.turns])
        rows.append((mode, {
            "games": n,
            "place": round(100.0 * with_cycle / max(1, n), 1),
            "place_ci": (0, 0),
            "top4": round(total_cycles / max(1, n), 2),
            "first": "",
        }))
    print("columns: place = turns with at least one cycle %, top4 = cycles per turn")
    print(table(rows))
    print(f"Era: {era[0]} of {era[1]} turns with at least one cycle "
          f"({100.0 * era[0] / max(1, era[1]):.1f}%), {era[2] / max(1, era[1]):.2f} cycles a turn")
    n = len(batches[: args.turns])
    result = {"turns": n, "era_with": era[0], "era_turns": era[1],
              "era_cycles": era[2]}
    for label, row in rows:
        result[f"{label}_with"] = row["place"] / 100.0
        result[f"{label}_cycles"] = row["top4"]
    return result


def era_cycles(dir_: Path, from_turn: int = 6):
    """Era's own cycles from the action files: a card bought or played and sold
    within the same turn."""
    import json
    turns = with_cycle = total = 0
    for path in sorted(dir_.glob("*.json")):
        try:
            raw = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        for turn in raw.get("turns", []):
            if int(turn.get("turn", 0)) < from_turn:
                continue
            steps = turn.get("actions", [])
            if not steps:
                continue
            bought, played, sold = set(), set(), set()
            for st in steps:
                kind = st.get("step")
                card = (st.get("card") or {})
                cid = card.get("card_id") or card.get("name")
                if kind == "buy" and cid:
                    bought.add(cid)
                elif kind == "play" and cid:
                    played.add(cid)
                elif kind == "sell" and cid:
                    sold.add(cid)
            turns += 1
            cycles = (bought | played) & sold
            with_cycle += bool(cycles)
            total += len(cycles)
    return with_cycle, turns, total


def placement(net, args, dev, boards) -> None:
    rows = []
    for weight in args.weights:
        t = time.time()
        rec, buys = play(net, args.games, args.seed, dev, boards=boards,
                         gen_seed=args.seed, track=True)
        s = summarize(rec, buys)
        rows.append((f"plain (w {weight:g})", s))
        print(f"plain done in {time.time() - t:.1f} s", flush=True)
        print(f"  plain (w {weight:g}): place {s['place']} {s['place_ci']}, "
              f"top4 {s['top4']}, first {s['first']}", flush=True)
        t = time.time()
        searcher = Searcher(net, dev=dev, mode="search", playouts=args.playouts,
                            fights=args.fights, fight_weight=weight, seed=args.seed,
                            replan=args.replan, playout_temperature=args.playout_temperature,
                            playout_actions=args.playout_actions, margin=args.margin)
        rec, buys = play(net, args.games, args.seed, dev, boards=boards,
                         searcher=searcher, gen_seed=args.seed, track=True)
        s = summarize(rec, buys, searcher)
        rows.append((f"search w {weight:g}", s))
        print(f"search w {weight:g} done in {time.time() - t:.1f} s", flush=True)
        print(f"  search w {weight:g}: place {s['place']} {s['place_ci']}, "
              f"top4 {s['top4']}, first {s['first']}, cycles/turn {s['cycles_per_turn']}, "
              f"{s['sec_per_decision']} s/decision", flush=True)
    print(table(rows))


# ---- main ----------------------------------------------------------------


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--ckpt", default=str(DEFAULT_CKPT), help="the network")
    ap.add_argument("--games", type=int, default=200, help="games per run")
    ap.add_argument("--seed", type=int, default=11)
    ap.add_argument("--playouts", type=int, default=1,
                    help="playouts per candidate (greedy playouts are deterministic, 1 is enough)")
    ap.add_argument("--playout-temperature", type=float, default=0.0,
                    help="the network's temperature after a candidate's first move (0 = greedy)")
    ap.add_argument("--playout-actions", type=int, default=60,
                    help="the most shop actions one playout takes")
    ap.add_argument("--margin", type=float, default=0.0,
                    help="a candidate must beat the network's own choice by this value margin")
    ap.add_argument("--fights", type=int, default=20, help="real boards per playout")
    ap.add_argument("--fight-weight", type=float, default=1.0,
                    help="weight of the expected Health change against the value "
                         "(1 = a hero's 30 Health is one value step)")
    ap.add_argument("--weights", type=float, nargs="*", default=[0.0, 0.5, 1.0, 2.0],
                    help="the fight weights the placement table runs")
    ap.add_argument("--replan", choices=["every", "turn", "cycle"], default="every",
                    help="when the real-trade judge runs: every shop decision (the "
                         "brief's default), once a turn, or never (see train/search.py)")
    ap.add_argument("--real-boards", default=str(ROOT / "boards" / "real"))
    ap.add_argument("--from-turn", type=int, default=6, help="the cycle-rate test")
    ap.add_argument("--positions", type=int, default=40, help="Blue Shell positions to collect")
    ap.add_argument("--verbose", action="store_true", help="print one position in full")
    ap.add_argument("--turns", type=int, default=200, help="Era turns the cycle-rate test plays")
    ap.add_argument("--vram-gb", type=float, default=0.0,
                    help="cap this process at this much GPU memory")
    ap.add_argument("--old-boards", choices=["keep", "drop"], default="keep",
                    help="what an opponent's last-seen board is worth (ppo.py's)")
    ap.add_argument("--force-dark-gift", action=argparse.BooleanOptionalAction, default=True,
                    help="press Dark Discovery on turns 3 to 5 (the r1106 run trained "
                         "with it, and its evals force it: ppo.force_mask)")
    ap.add_argument("--force-turn2-level", action="store_true",
                    help="ppo.py's --force-turn2-level, for a run trained with it")
    ap.add_argument("--shell-test", action="store_true")
    ap.add_argument("--warden-test", action="store_true")
    ap.add_argument("--cycle-rate", action="store_true")
    ap.add_argument("--placement", action="store_true")
    args = ap.parse_args()

    dev = "cuda" if torch.cuda.is_available() else "cpu"
    if dev == "cuda" and args.vram_gb > 0:
        torch.cuda.set_per_process_memory_fraction(args.vram_gb / 16.3)
        print(f"GPU capped at {args.vram_gb} GB (Era is at the PC)")
    ppo.FORCE_DARK = args.force_dark_gift
    ppo.FORCE_TURN2 = args.force_turn2_level
    ppo.OLD_BOARDS = args.old_boards
    net = load(args.ckpt, dev)
    net.eval()
    print(f"{args.ckpt}: {dev}")
    boards = None
    if not args.cycle_rate:
        specs, unknown = real_boards.load(Path(args.real_boards))
        boards = specs
        print(f"real boards: {len(specs)} ({len(unknown)} unknown minions)")
    if args.shell_test:
        shell_test(net, args, dev, boards)
    elif args.warden_test:
        warden_test(net, args, dev)
    elif args.cycle_rate:
        cycle_rate(net, args, dev)
    else:
        placement(net, args, dev, boards)


if __name__ == "__main__":
    main()
