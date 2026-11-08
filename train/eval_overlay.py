"""The comp overlay against seven real boards (docs/briefs/comp-overlay-1.md).

One learner seat plays r1106-keep against seven real-board seats. With the
overlay on, the Rust rules force any short line that dominates the position
and take offered enablers/payoffs, and add the logit bonus `B` for buying or
playing a payoff (`B/2` for a filler). With `B = 0` the same loop is the
plain network, and a separate 512-game check reproduces the baseline exactly.

    .venv/bin/python train/eval_overlay.py --games 2048 --bonus 0 2 4 8

Output: one table per B, with the Shell and enabler counts, lines forced, the
active games' comp share and place, and steps per second. `--no-dominance`
runs the same seeds without the line search; `--lines` records the line cards
and 10 examples. `--out DIR` writes `report.md` and `results.json`.
"""

from __future__ import annotations

import argparse
import json
import time
import tomllib
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
import torch

import bgpy
import eval_search
import ppo
import real_boards
from model import load
from search import FIRST, KIND_OF, K_BUY, K_PLAY, K_SELL, play_parts

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CKPT = ROOT / "train" / "runs" / "r1106-keep" / "ckpt.pt"
DEFAULT_COMPS = ROOT / "data" / "comps" / "comps.toml"
P = bgpy.PLAYERS


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
    shell_legal_slot: set[int] = field(default_factory=set)
    shell_taken: set[int] = field(default_factory=set)
    enabler_offered: bool = False
    enabler_taken: bool = False
    active_turn: int | None = None
    lines: int = 0
    line_actions: int = 0
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
    if f[2]:
        s.enabler_offered = True
    if f[3]:
        s.enabler_taken = True
    if f[4] and s.active_turn is None:
        s.active_turn = turn


stats_of.games = []


# ---- reading the observation for the dominance numbers -------------------


def card_at(obs, start, slot):
    """The card id in one slot of the observation, or -1 for an empty slot."""
    v = int(obs[start + slot * bgpy.SLOT_FEATURES + bgpy.SLOT_CARD_ID])
    return v - 1 if v > 0 else -1


def slot_count(obs, start, n):
    """How many slots of a zone are occupied."""
    return sum(
        1 for s in range(n)
        if obs[start + s * bgpy.SLOT_FEATURES + bgpy.SLOT_PRESENT] > 0
    )


def action_card(obs, a, names):
    """The card an action names, by its pre-action observation, or None."""
    kind = int(KIND_OF[a])
    if kind == K_BUY:
        cid = card_at(obs, bgpy.SHOP_START, a)
    elif kind == K_SELL:
        cid = card_at(obs, bgpy.BOARD_START, a - FIRST["sell"])
    elif kind == K_PLAY:
        hand, _ = play_parts(a)
        cid = card_at(obs, bgpy.HAND_START, hand)
    else:
        return None
    return names[cid] if 0 <= cid < len(names) else None


def describe_text(desc, names):
    """One position as the search describes it, in a readable line."""
    gold, tier, health, armor, upgrade, free, frozen, hand, shop, board = desc

    def minion(m):
        name = names[m[0]] if 0 <= m[0] < len(names) else str(m[0])
        return f"{name} {m[1]}/{m[2]}"

    def spell(m):
        name = names[m[0]] if 0 <= m[0] < len(names) else str(m[0])
        return name

    parts = [
        f"gold {gold}, tier {tier}, {health} Health",
        "board [" + ", ".join(minion(m) for m in board) + "]",
        "hand [" + ", ".join(spell(m) for m in hand) + "]",
        "Tavern [" + ", ".join(minion(m) for m in shop) + "]",
    ]
    return "; ".join(parts)


def env_position(env, i, names):
    """Describe game i's current position, or None when it has just ended."""
    try:
        return describe_text(env.search_copy(i).describe(0), names)
    except Exception:
        return None


def play(net, n_games, seed, dev, boards, comps_path=None, bonus=0.0,
         gen_seed=0, dominance=True, collect_lines=False, names=None):
    """Play `n_games` with the overlay on (`bonus > 0`) or off.

    Off uses the exact eval_search.play path, so it reproduces the 5.769
    baseline on this build. On uses the same loop with the overlay call;
    `dominance=False` keeps the comp bonuses but turns the dominance layer
    off. `collect_lines` reads the observations for the Shell and line
    numbers (off by default: it is measurement, not play). Returns
    (records, game_stats, timing)."""
    t = time.time()
    if bonus <= 0.0:
        records, _ = eval_search.play(
            net, n_games, seed, dev, lineup=(1, 0, 0, 0, 0, 7),
            boards=boards, gen_seed=gen_seed, track=False,
        )
        return records, [], {"seconds": time.time() - t, "overlay_ns": 0, "steps": 0}

    n = min(n_games, 512)
    env = bgpy.VecEnv(n, seed, [1, 0, 0, 0, 0, 7], real_boards=boards,
                      old_boards=ppo.OLD_BOARDS,
                      real_lobby_filter=ppo.REAL_LOBBY_FILTER)
    env.set_overlay(str(comps_path), float(bonus), dominance)
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
    observe_ns = 0
    sample_ns = 0
    step_ns = 0
    steps = 0
    if collect_lines and names is None:
        names = net_card_names()
    shell_id = names.index("Tortollan Blue Shell") if collect_lines and \
        "Tortollan Blue Shell" in names else -1
    line_stats = {"lines": 0, "actions": 0, "cards": Counter(), "examples": []}
    open_lines: dict[int, dict] = {}

    def close_line(i):
        ol = open_lines.pop(i, None)
        if ol is None:
            return
        ol["stats"].line_actions += len(ol["actions"])
        line_stats["actions"] += len(ol["actions"])
        if len(line_stats["examples"]) < 2000:
            end = env_position(env, i, names)
            line_stats["examples"].append({
                "source": ol["source"],
                "turn": ol["turn"],
                "actions": list(ol["actions"]),
                "names": list(ol["names"]),
                "gold_before": ol["gold"],
                "gold_after": int(b.obs[i, bgpy.SCALAR_GOLD]),
                "before": ol["start_desc"],
                "after": end,
            })

    while done < n_games:
        t0 = time.perf_counter_ns()
        env.observe(b.obs, b.mask, b.seat, b.kind)
        env.remember_boards()
        observe_ns += time.perf_counter_ns() - t0
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
            if not collect_lines:
                continue
            st = game_stats[i]
            obs = b.obs[i]
            gold = int(obs[bgpy.SCALAR_GOLD])
            lost = obs[bgpy.SCALAR_LOST_LAST_COMBAT] > 0.5
            hand_n = slot_count(obs, bgpy.HAND_START, bgpy.HAND)
            board_n = slot_count(obs, bgpy.BOARD_START, bgpy.BOARD)
            if lost and shell_id >= 0:
                shop_shell = next(
                    (s for s in range(bgpy.SHOP)
                     if card_at(obs, bgpy.SHOP_START, s) == shell_id
                     and obs[bgpy.SHOP_START + s * bgpy.SLOT_FEATURES
                             + bgpy.SLOT_COST] <= gold),
                    None,
                )
                if shop_shell is not None and hand_n < bgpy.HAND:
                    st.shell_legal.add(turn)
                    if board_n < bgpy.BOARD:
                        st.shell_legal_slot.add(turn)
            f = b.flags[i, seat]
            a = int(b.forced[i, seat])
            if f[1] and a >= 0:
                if f[0] or i not in open_lines:
                    close_line(i)
                    open_lines[i] = {
                        "turn": turn,
                        "actions": [],
                        "names": [],
                        "start_desc": env_position(env, i, names),
                        "gold": gold,
                        "source": None,
                        "stats": st,
                        "lost": lost,
                    }
                    line_stats["lines"] += 1
                    st.lines += 1
                ol = open_lines[i]
                ol["actions"].append(a)
                name = action_card(obs, a, names)
                ol["names"].append(name)
                if name:
                    line_stats["cards"][name] += 1
                    if name == "Tortollan Blue Shell" and lost:
                        st.shell_taken.add(turn)
                if ol["source"] is None:
                    ol["source"] = name
            elif i in open_lines:
                close_line(i)
        t0 = time.perf_counter_ns()
        actions = sample_bonus(
            net, b.obs, b.mask, gen, dev,
            bonus=b.bonus[np.arange(n), b.seat],
            forced=b.forced[np.arange(n), b.seat],
        )
        sample_ns += time.perf_counter_ns() - t0
        turn = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
        turns += (turn > np.maximum(turns, 0)) & (b.kind == bgpy.LEARNER)
        kinds = b.kinds.copy()
        t0 = time.perf_counter_ns()
        env.step(actions, b.places, b.tier_turns, b.tribes, b.reset)
        step_ns += time.perf_counter_ns() - t0
        if b.reset.any():
            finals = env.final_boards()
            for i in np.nonzero(b.reset)[0]:
                open_lines.pop(int(i), None)
                seats = np.nonzero(kinds[i] == bgpy.LEARNER)[0]
                if len(seats) != 1:
                    raise SystemExit("the overlay eval expects one learner seat")
                s = int(seats[0])
                st = game_stats[i]
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
    timing = {"seconds": time.time() - t, "overlay_ns": overlay_ns, "steps": steps,
              "observe_ns": observe_ns, "sample_ns": sample_ns, "step_ns": step_ns}
    if collect_lines:
        timing["line_stats"] = line_stats
    return records, completed, timing

def summarize(records, game_stats, enabler_ids, comp_ids, off_places_by_enabler=None):
    places = np.array([r[0] for r in records], float)
    n = len(places)
    rng = np.random.default_rng(7)
    boots = [places[rng.integers(0, n, n)].mean() for _ in range(2000)]
    active = [s for s in game_stats if s.active_turn is not None]
    comp_boards = [
        s for s in active
        if any(x in enabler_ids for x in s.final)
        and sum(1 for x in s.final if x in comp_ids) >= 2
    ]
    return {
        "games": n,
        "place": round(float(places.mean()), 3),
        "place_ci": (round(float(np.percentile(boots, 2.5)), 3),
                     round(float(np.percentile(boots, 97.5)), 3)),
        "top4": round(float((places <= 4).mean()), 3),
        "first": round(float((places == 1).mean()), 3),
        "shell_legal": sum(len(s.shell_legal) for s in game_stats),
        "shell_legal_slot": sum(len(s.shell_legal_slot) for s in game_stats),
        "shell_taken": sum(len(s.shell_taken) for s in game_stats),
        "lines": sum(s.lines for s in game_stats),
        "line_actions": sum(s.line_actions for s in game_stats),
        "enabler_offered": sum(1 for s in game_stats if s.enabler_offered),
        "enabler_taken": sum(1 for s in game_stats if s.enabler_taken),
        "active": len(active),
        "active_turn": round(float(np.mean([s.active_turn for s in active])), 2) if active else 0.0,
        "active_place": round(float(np.mean([s.place for s in active])), 3) if active else 0.0,
        "comp_boards": len(comp_boards),
        "comp_share": round(len(comp_boards) / len(active), 3) if active else 0.0,
    }


def load_comps(path):
    """The enabler and payoff/filler card names of the first comp with one."""
    data = tomllib.loads(Path(path).read_text())
    comps = data.get("comp", [])
    for c in comps:
        if c.get("enabler"):
            return c["enabler"], c.get("payoff", []) + c.get("filler", []), c.get("name", "?")
    raise SystemExit(f"{path}: no comp has an enabler")


def names_to_ids(names, wanted):
    out = set()
    for name in wanted:
        if name in names:
            out.add(names.index(name))
        else:
            raise SystemExit(f"card {name!r} is not in the card set")
    return out


def action_text(a, name):
    """One forced action in words."""
    kind = int(KIND_OF[a])
    if kind == K_BUY:
        return f"buy {name}"
    if kind == K_SELL:
        return f"sell {name}"
    if kind == K_PLAY:
        return f"play {name}"
    return f"action {a}"


def pick_examples(examples, n=10):
    """Up to n lines, one per source card first, then the rest."""
    chosen, seen = [], set()
    for i, e in enumerate(examples):
        if e["source"] and e["source"] not in seen:
            chosen.append(i)
            seen.add(e["source"])
            if len(chosen) >= n:
                return [examples[j] for j in chosen]
    for i in range(len(examples)):
        if i not in chosen:
            chosen.append(i)
            if len(chosen) >= n:
                break
    return [examples[j] for j in chosen]


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--ckpt", default=str(DEFAULT_CKPT))
    ap.add_argument("--comps", default=str(DEFAULT_COMPS))
    ap.add_argument("--games", type=int, default=2048)
    ap.add_argument("--bonus", type=float, nargs="*", default=[0, 2, 4, 8],
                    help="B values; 0 is the overlay off")
    ap.add_argument("--seed", type=int, default=14606)
    ap.add_argument("--real-boards", default=str(ROOT / "boards" / "real"))
    ap.add_argument("--out", default=None)
    ap.add_argument("--vram-gb", type=float, default=4.0,
                    help="cap this process's GPU memory (4 while the PPO run trains)")
    ap.add_argument("--real-lobby-filter", action="store_true",
                    help="a real seat draws only boards whose minions fit this game's lobby")
    ap.add_argument("--no-dominance", action="store_true",
                    help="keep the comp bonuses but turn the dominance layer off")
    ap.add_argument("--lines", action="store_true",
                    help="record the forced lines, the Shell rate and 10 examples")
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
    print(f"{args.ckpt}: {dev}; {len(specs)} real boards ({len(unknown)} unknown minions)")
    print("r1108-fresh-cyc is training; the overlay cost below is relative, not a clean speed number")

    enabler_names, comp_names, comp_label = load_comps(args.comps)
    names = net_card_names()
    enabler_ids = names_to_ids(names, enabler_names)
    comp_ids = names_to_ids(names, comp_names)
    results = {}
    for b in args.bonus:
        t = time.time()
        records, game_stats, timing = play(
            net, args.games, args.seed, dev, specs, args.comps, b,
            gen_seed=args.seed, dominance=not args.no_dominance,
            collect_lines=args.lines, names=names,
        )
        summary = summarize(records, game_stats, enabler_ids, comp_ids)
        summary["seconds"] = round(time.time() - t, 2)
        summary["steps"] = int(timing["steps"])
        summary["steps_per_second"] = (
            round(timing["steps"] / timing["seconds"], 1) if timing["seconds"] else 0.0
        )
        summary["overlay_ms_per_step"] = (
            round(timing["overlay_ns"] / timing["steps"] / 1e6, 4) if timing["steps"] else 0.0
        )
        summary["overlay_share"] = (
            round(100.0 * timing["overlay_ns"] / (timing["seconds"] * 1e9), 2) if timing["steps"] else 0.0
        )
        for key in ("observe_ns", "sample_ns", "step_ns"):
            summary[key.replace("_ns", "_ms_per_step")] = (
                round(timing.get(key, 0) / timing["steps"] / 1e6, 4) if timing["steps"] else 0.0
            )
        summary["label"] = f"B {b:g}" if b > 0 else "off"
        if "line_stats" in timing:
            ls = timing["line_stats"]
            summary["line_stats"] = {
                "lines": ls["lines"],
                "actions": ls["actions"],
                "cards": dict(ls["cards"]),
                "examples": ls["examples"],
            }
        results[b] = summary
        if b == 0:
            print(
                f"off: place {summary['place']} {summary['place_ci']}, "
                f"top4 {summary['top4']:.1%}, {summary['seconds']} s (rules not recorded)",
                flush=True,
            )
        else:
            legal = summary["shell_legal_slot"]
            rate = 0 if not legal else 100 * summary["shell_taken"] / legal
            all_rate = 0 if not summary["shell_legal"] else \
                100 * summary["shell_taken"] / summary["shell_legal"]
            lines = summary.get("line_stats", {}).get("lines", summary["lines"])
            print(
                f"{summary['label']}: place {summary['place']} {summary['place_ci']}, "
                f"top4 {summary['top4']:.1%}, shell {summary['shell_taken']}/{legal} "
                f"({rate:.1f}%, all legal {summary['shell_taken']}/{summary['shell_legal']} "
                f"{all_rate:.1f}%), lines {lines}, enabler offered/taken "
                f"{summary['enabler_offered']}/{summary['enabler_taken']}, "
                f"active {summary['active']} (turn {summary['active_turn']}), "
                f"comp {summary['comp_share']:.1%} of active, active place {summary['active_place']}, "
                f"{summary['seconds']} s, {summary['steps_per_second']} steps/s",
                flush=True,
            )

    # The off check. The 200-action build's baseline was 5.867; removing the
    # cycle actions renormalises the 183-action policy, so this is the new
    # plain-network number.
    print("\n512-game overlay-off check (the 200-action baseline was 5.867):")
    rec, _ = eval_search.play(net, 512, args.seed, dev, lineup=(1, 0, 0, 0, 0, 7),
                              boards=specs, gen_seed=args.seed, track=False)
    s = eval_search.summarize(rec)
    print(f"  off 512: place {s['place']} {s['place_ci']}, top4 {s['top4']:.1%}")

    print("\n| B | place (95%) | top4 | Shell taken/legal (free slot) | Shell taken/all | lines | lines/game | enabler offered/taken | active | active turn | comp boards | active place | steps/s |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for b in args.bonus:
        r = results[b]
        if b == 0:
            print(
                f"| {r['label']} | {r['place']} ({r['place_ci'][0]} to {r['place_ci'][1]}) | "
                f"{r['top4']:.1%} | - | - | - | - | - | - | - | - | - | - |"
            )
            continue
        slot = r["shell_legal_slot"]
        rate = 0 if not slot else 100 * r["shell_taken"] / slot
        all_rate = 0 if not r["shell_legal"] else 100 * r["shell_taken"] / r["shell_legal"]
        lines = r.get("line_stats", {}).get("lines", r["lines"])
        print(
            f"| {r['label']} | {r['place']} ({r['place_ci'][0]} to {r['place_ci'][1]}) | "
            f"{r['top4']:.1%} | {r['shell_taken']}/{slot} ({rate:.1f}%) | "
            f"{r['shell_taken']}/{r['shell_legal']} ({all_rate:.1f}%) | "
            f"{lines} | {lines / max(r['games'], 1):.3f} | "
            f"{r['enabler_offered']}/{r['enabler_taken']} | {r['active']} | {r['active_turn']} | "
            f"{r['comp_boards']} ({r['comp_share']:.1%}) | {r['active_place']} | "
            f"{r['steps_per_second']} |"
        )
    print("\nOff matching: can't. The off run is the plain eval_search loop and records no enabler offer.")
    print("\nSpeed and overlay call cost (B > 0):")
    for b in args.bonus:
        r = results[b]
        if r["overlay_ms_per_step"]:
            print(f"  B {b:g}: {r['steps_per_second']} steps/s, "
                  f"{r['overlay_ms_per_step']} ms/step in the overlay call, "
                  f"{r['overlay_share']}% of the loop; observe "
                  f"{r['observe_ms_per_step']} ms, network "
                  f"{r['sample_ms_per_step']} ms, step {r['step_ms_per_step']} ms")
    if args.lines:
        for b in args.bonus:
            r = results[b]
            ls = r.get("line_stats")
            if not ls:
                continue
            print(f"\nDominating lines at B {b:g}: {ls['lines']} lines, "
                  f"{ls['actions']} forced actions, "
                  f"{ls['lines'] / max(r['games'], 1):.3f} per game")
            top = Counter(ls["cards"]).most_common(15)
            print("  top 15 cards: " + ", ".join(f"{name} {n}" for name, n in top))
            for k, e in enumerate(pick_examples(ls["examples"], 10), 1):
                acts = " -> ".join(
                    action_text(a, n) for a, n in zip(e["actions"], e["names"])
                )
                print(f"\n  line {k}: turn {e['turn']}, source {e['source']}, "
                      f"gold {e['gold_before']} -> {e['gold_after']}")
                print(f"    before: {e['before'] or '?'}")
                print(f"    after:  {e['after'] or '?'}")
                print(f"    actions: {acts}")

    if args.out:
        out = Path(args.out)
        out.mkdir(parents=True, exist_ok=True)
        clean = {str(k): v for k, v in results.items()}
        (out / "results.json").write_text(json.dumps(clean, indent=2) + "\n")
        lines = ["# Comp overlay eval", ""]
        for b in args.bonus:
            r = results[b]
            lines.append(f"- {r['label']}: place {r['place']} {r['place_ci']}, top4 {r['top4']:.1%}, "
                         f"active {r['active']}, comp share {r['comp_share']:.1%}")
        (out / "report.md").write_text("\n".join(lines) + "\n")


def net_card_names():
    """The card names by id, from the bundled data (no env needed)."""
    return bgpy.VecEnv(1, 0, [8, 0, 0, 0, 0, 0], real_boards=None).card_names()


if __name__ == "__main__":
    main()
