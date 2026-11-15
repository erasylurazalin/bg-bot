"""The bot plays Era's turns, with Era's luck (2026-11-07, docs/notes/training-plans.md
"THE PLAN", "The main measuring stick").

For every logged shop turn of Era's from `--from-turn` on, the bot is put in
exactly Era's position at the start of the turn and plays it: once with the
plain network (`--players plain`) and once with the search of train/search.py
(`--players search`), each `--repeats` times with its own seed. As long as the
bot takes Era's own logged steps (same engine actions, in the same order), the
state before every decision is seated from the log, so the shop, the Tavern a
Refresh deals it and the Discover offers are the log's, never the engine's
draw (`bgpy.EraTurnPath`, `bgcore::shop_replay::seat_step`). The first action
that is not Era's leaves her path, and from then on the engine draws (seeded
per repeat).

Both end-of-turn positions are judged by fighting them, with the same boards
and the same seeds:

  * Era's real next opponent: the opponent board she fought after this turn,
    from `boards/real` (the same game, the same combat). The headline number.
  * 20 real boards of that turn, evenly spread (the search's own judging rule).
  * the network's value of the position.

Era's own end-of-turn position is the same machinery with a player that always
plays Era's next logged step (`bgpy.EraTurnPath.play_era`); the fidelity check
is whether that position is what the log shows at the end of the turn.

    .venv/bin/python train/era_turns.py --name 2026-11-07
    .venv/bin/python train/era_turns.py --players plain --turns 20
    .venv/bin/python train/era_turns.py --vram-gb 6      # if pcbusy says ON

Output goes to train/runs/era-turns-<name>/: `turns.tsv` (one row per turn and
player), `report.md` (per player, per comp and per turn bucket), `skipped.tsv`
(the turns the replay cannot seat, and why) and `turns.jsonl` (the raw per-turn
records, which `--resume` picks up after a crash).
"""

from __future__ import annotations

import argparse
import json
import time
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np
import torch

import bgpy
import ppo
import real_boards
from model import load
from search import Searcher

import eval_search

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CKPT = ROOT / "train" / "runs" / "r1106-keep" / "ckpt.pt"
COMPS = ROOT / "train" / "runs" / "comps-era-1106b"
BUCKETS = ((3, 5, "3-5"), (6, 8, "6-8"), (9, 11, "9-11"), (12, 10**6, "12+"))


def bucket_of(turn: int) -> str:
    for lo, hi, name in BUCKETS:
        if lo <= turn <= hi:
            return name
    return "12+"


# ---- the labels ----------------------------------------------------------


def comp_of(replay, fights) -> tuple[str, str]:
    """The comp the comps run gave the game, else the top tribe of Era's final
    board (the leaderboard's rule); and the family for a coarser table (the
    first tribe the comps run listed, else the same top tribe)."""
    path = COMPS / f"game-{replay.file}"
    comp = family = None
    if path.exists():
        try:
            answer = json.loads(path.read_text()).get("answer") or {}
        except (OSError, ValueError):
            answer = {}
        comp = (answer.get("comp") or "").strip() or None
        tribes = answer.get("tribes") or []
        if tribes:
            family = str(tribes[0]).strip().capitalize()
    tribe = fights.final_tribe(replay.source, replay.game)
    if comp is None:
        comp = "mixed" if tribe == "mixed" else tribe
        comp = comp if comp in ("mixed", "none") else comp.capitalize()
    if family is None:
        family = "mixed" if tribe == "mixed" else tribe.capitalize()
    return comp, family


# ---- one seat's turn -----------------------------------------------------


def plain_action(net, path, gen, dev, obs, mask) -> int:
    """The plain network's action, sampled at temperature 1 with ppo.py's
    forced mask exactly as eval_search.py's plain baseline does (the run's
    forced Dark Gift)."""
    path.observe(obs, mask)
    return int(eval_search.sample(net, obs[None], mask[None], gen, dev)[0])


def play_bot(net, path, player, searcher, gen, dev, args, planner=None) -> dict:
    """Play one turn of one repeat and say how it went: the actions, the card
    events and where the turn left Era's path."""
    obs = np.zeros(bgpy.OBS_SIZE, np.float32)
    mask = np.zeros(bgpy.N_ACTIONS, bool)
    actions = 0
    error = None
    try:
        while path.state != 2 and path.actions_left > 1 and actions < args.max_actions:
            if player == "plain":
                a = plain_action(net, path, gen, dev, obs, mask)
            elif player == "planner":
                a = planner.action(path)
            else:
                a = searcher.decide_batch(path.batch(), key=("era", path.file, path.turn))
            path.act(a)
            actions += 1
    except (ValueError, IndexError, RuntimeError) as e:
        # One turn's engine refusing an action must not end the run: the turn
        # is recorded with what it got to.
        error = str(e)
    path.finish()
    return {
        "actions": actions,
        "error": error,
        "steps_on_path": path.steps_on_path,
        "logged_steps": path.logged_steps,
        "left_at": int(path.left_at),
        "left_why": path.left_why,
        "missing": path.missing,
        "bought": list(path.bought),
        "played": list(path.played),
        "sold": list(path.sold),
        "levelled": bool(path.levelled),
    }


def cycles(path_summary, names) -> list[str]:
    """The cards bought or played and sold in the same turn, by name."""
    bought = set(path_summary["bought"])
    played = set(path_summary["played"])
    sold = set(path_summary["sold"])
    return sorted({names[c] for c in (bought | played) & sold if c < len(names)})


def board_text(desc, names) -> str:
    """A board as `Name a/h` lines, golden marked."""
    out = []
    for card, attack, health, golden in desc[9]:
        name = names[card] if card < len(names) else str(card)
        out.append(f"{name}{' (golden)' if golden else ''} {attack}/{health}")
    return ", ".join(out) if out else "(empty)"


def value_of(net, batch, dev) -> float:
    obs = np.zeros((1, bgpy.OBS_SIZE), np.float32)
    mask = np.zeros((1, bgpy.N_ACTIONS), bool)
    state = np.zeros(1, np.int8)
    batch.observe_all(obs, mask, state)
    with torch.no_grad():
        _, value = net(torch.from_numpy(obs).to(dev))
    return float(value[0])


def judge(fights, batch, names, net, dev, turn, opponent, typical, seed, args):
    """Fight a position against Era's real next opponent and the turn's
    typical boards, and read the network's value of it."""
    out = {
        "opp": None,
        "typ": None,
        "value": round(value_of(net, batch, dev), 5),
        "board": board_text(batch.describe(0), names),
    }
    if opponent >= 0:
        win, tie, loss, health = fights.fight(batch, 0, [opponent] * args.fights, seed)
        out["opp"] = [win, tie, loss, health / max(1, args.fights)]
    if typical:
        win, tie, loss, health = fights.fight(batch, 0, typical, seed ^ 0x7119)
        out["typ"] = [win, tie, loss, health / max(1, len(typical))]
    return out


# ---- the run -------------------------------------------------------------


def load_done(path: Path) -> dict:
    """The per-turn records `--resume` already has, by (file, turn)."""
    done = {}
    if not path.exists():
        return done
    for line in path.read_text().splitlines():
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        done[(rec["file"], rec["turn"])] = rec
    return done


def unloaded_turns(bad: list[str], actions: Path, from_turn: int) -> list[dict]:
    """The turns of an action file the loader cannot build, counted from the
    file's own JSON: `bgcore::shop_replay::parse_action_files` refuses the whole
    file, so no turn of it can be seated."""
    out = []
    for line in bad:
        name = line.split(": ")[0]
        try:
            raw = json.loads((actions / name).read_text())
        except (OSError, ValueError):
            out.append({"file": name, "turn": 0,
                        "why": f"the file does not load and cannot be read: {line}"})
            continue
        for turn in raw.get("turns", []):
            if int(turn.get("turn", 0)) >= from_turn and turn.get("actions"):
                out.append({"file": name, "turn": int(turn["turn"]),
                            "why": f"the file does not load: {line}"})
    return out


def run(net, dev, args, replays, bad, fights, out: Path, boards=None) -> tuple[list[dict], list[dict]]:
    names = None
    for replay in replays:
        turns = replay.turns(args.from_turn)
        if turns:
            names = replay.path(turns[0], args.seed, True).card_names()
            break
    if names is None:
        raise SystemExit("no seatable turn to play")

    planners = {}
    skipped = unloaded_turns(bad, Path(args.actions), args.from_turn)
    # The turns that cannot be seated at all: the path's own seating says why.
    for replay in replays:
        for turn in replay.turns(args.from_turn):
            try:
                replay.path(turn, args.seed, True)
            except ValueError as e:
                skipped.append({"file": replay.file, "turn": turn, "why": str(e)})
    seatable = []
    if args.turns or args.sample:
        for replay in replays:
            for turn in replay.turns(args.from_turn):
                if not any(s["file"] == replay.file and s["turn"] == turn for s in skipped):
                    seatable.append((replay, turn))
    if args.sample:
        # N turns spread evenly over the seatable turns, every k-th, so the
        # sample covers all games and comps instead of the first N files.
        n = min(args.sample, len(seatable))
        chosen = [seatable[int((i + 0.5) * len(seatable) / n)] for i in range(n)] if n else []
        wanted = {(r.file, t) for r, t in chosen}
        print(f"sample: {len(wanted)} of {len(seatable)} seatable turns, evenly spread",
              flush=True)
    elif args.turns:
        wanted = {(r.file, t) for r, t in seatable[: args.turns]}
    else:
        wanted = None

    raw = out / "turns.jsonl"
    done = load_done(raw) if args.resume else {}
    records = list(done.values())
    skip_keys = {(s["file"], s["turn"]) for s in skipped}
    todo = 0
    for replay in replays:
        for turn in replay.turns(args.from_turn):
            if (replay.file, turn) not in done and (replay.file, turn) not in skip_keys:
                todo += 1
    print(f"{len(replays)} games, {todo} turns to play"
          + (f" ({len(done)} already in {raw.name})" if done else ""), flush=True)

    gen = torch.Generator(device="cpu")
    searchers = {}
    t_start = time.time()
    played = 0
    with raw.open("a" if done else "w", encoding="utf-8") as sink:
        for replay in replays:
            for turn in replay.turns(args.from_turn):
                key = (replay.file, turn)
                if key in done:
                    continue
                if wanted is not None and key not in wanted:
                    continue
                if key in skip_keys:
                    continue
                t_turn = time.time()
                comp, family = comp_of(replay, fights)
                opponent = fights.opponent(replay.source, replay.game, turn)
                typical = fights.typical(turn, args.typical)
                seed = args.seed * 1_000_003 + turn

                # Era's own end-of-turn position: her logged steps, and the
                # seed that reproduces the log's end state when one does.
                era = None
                era_diffs = []
                era_error = None
                match_first = match_any = False
                for k in range(max(1, args.fidelity_seeds)):
                    try:
                        candidate = replay.path(turn, args.seed + k, True)
                        candidate.play_era()
                        diffs = candidate.diffs()
                    except ValueError as e:
                        era_error = str(e)
                        continue
                    if k == 0:
                        era, era_diffs, match_first = candidate, diffs, not diffs
                    if not diffs:
                        era, era_diffs, match_any = candidate, [], True
                        break
                if era is None:
                    skipped.append({
                        "file": replay.file, "turn": turn,
                        "why": f"Era's own line cannot be replayed: {era_error}",
                    })
                    continue
                era_desc = era.describe()
                era_batch = era.batch()
                era_judged = judge(fights, era_batch, names, net, dev, turn,
                                   opponent, typical, seed, args)
                era_cycles = cycles(
                    {"bought": era.bought, "played": era.played, "sold": era.sold}, names
                )
                record = {
                    "file": replay.file,
                    "turn": turn,
                    "comp": comp,
                    "family": family,
                    "bucket": bucket_of(turn),
                    "source": replay.source,
                    "game": replay.game,
                    "logged_steps": era.logged_steps,
                    "opponent_found": opponent >= 0,
                    "era": {
                        "match_first": match_first,
                        "match_any": match_any,
                        "diffs": era_diffs,
                        "judged": era_judged,
                        "gold": int(era_desc[0]),
                        "tier": int(era_desc[1]),
                        "cycles": era_cycles,
                        "levelled": bool(era.levelled),
                        "board": era_judged["board"],
                    },
                    "players": {},
                }
                for player in args.players:
                    repeats = []
                    for repeat in range(args.repeats):
                        if player == "search":
                            searcher = searchers.get(repeat)
                            if searcher is None:
                                searcher = Searcher(
                                    net, dev=dev, mode="search",
                                    playouts=args.playouts, fights=args.search_fights,
                                    fight_weight=args.fight_weight,
                                    seed=args.seed * 31 + repeat, replan=args.replan,
                                    playout_temperature=args.playout_temperature,
                                    playout_actions=args.playout_actions,
                                    margin=args.margin,
                                )
                                searcher.names = names
                                searcher.force = eval_search.forced
                                searchers[repeat] = searcher
                            gen.manual_seed((args.seed * 7919 + repeat * 104729 + turn) & 0xFFFF_FFFF)
                        else:
                            searcher = None
                            gen.manual_seed((args.seed * 7919 + repeat * 104729 + turn) & 0xFFFF_FFFF)
                        planner = None
                        if player == "planner":
                            planner = planners.get(repeat)
                            if planner is None:
                                planner = bgpy.Planner(
                                    args.planner, args.seed * 31 + repeat, boards
                                )
                                planners[repeat] = planner
                            planner.reset(
                                (args.seed * 7919 + repeat * 104729 + turn) & 0xFFFF_FFFF
                            )
                        path = replay.path(turn, args.seed + repeat * 101 + 1, True)
                        summary = play_bot(net, path, player, searcher, gen, dev, args, planner)
                        if planner is not None:
                            summary["comp"] = planner.comp
                            summary["committed_turn"] = planner.committed_turn
                            summary["shell_cycles"] = planner.shell_cycles
                        batch = path.batch()
                        judged = judge(fights, batch, names, net, dev, turn,
                                       opponent, typical, seed, args)
                        desc = path.describe()
                        summary.update({
                            "judged": judged,
                            "gold": int(desc[0]),
                            "tier": int(desc[1]),
                            "value": judged["value"],
                            "cycles": cycles(summary, names),
                            "board": judged["board"],
                        })
                        repeats.append(summary)
                    record["players"][player] = repeats
                sink.write(json.dumps(record) + "\n")
                sink.flush()
                records.append(record)
                played += 1
                elapsed = time.time() - t_start
                per = elapsed / played
                print(
                    f"[{played}/{todo}] {replay.file} turn {turn} "
                    f"({comp}) {time.time() - t_turn:.1f}s, "
                    f"{per:.1f}s/turn, left {todo - played} ({per * (todo - played) / 60:.0f} min)",
                    flush=True,
                )
    if searchers:
        for repeat, searcher in searchers.items():
            stats = searcher.stats()
            print(f"search repeat {repeat}: {stats['decisions']} decisions, "
                  f"{stats['seconds_per_decision']:.3f} s/decision", flush=True)
    return records, skipped


# ---- the tables ----------------------------------------------------------


def mean(values):
    values = [v for v in values if v is not None]
    return sum(values) / len(values) if values else None


def fmt(value, digits=3, scale=1.0, suffix=""):
    if value is None:
        return "-"
    return f"{value * scale:.{digits}f}{suffix}"


def turns_with(records, player):
    """The records that have this player's repeat records."""
    return [r for r in records if player in r["players"]]


def pooled_opp(records, player, era=False):
    """The win/tie/loss counts and Health change against Era's real next
    opponent, pooled over turns (each turn contributes `--fights` fights)."""
    win = tie = loss = 0
    health = 0.0
    turns = 0
    for record in records:
        if not record["opponent_found"]:
            continue
        if era:
            opp = record["era"]["judged"]["opp"]
            if opp is None:
                continue
            win, tie, loss = win + opp[0], tie + opp[1], loss + opp[2]
            health += opp[3]
            turns += 1
            continue
        fights = [rep["judged"]["opp"] for rep in record["players"].get(player, [])
                  if rep["judged"]["opp"] is not None]
        if not fights:
            continue
        win += mean([f[0] for f in fights])
        tie += mean([f[1] for f in fights])
        loss += mean([f[2] for f in fights])
        health += mean([f[3] for f in fights])
        turns += 1
    return win, tie, loss, health, turns


def pooled_typ(records, player, era=False):
    win = loss = 0
    health = 0.0
    turns = 0
    for record in records:
        if era:
            typ = record["era"]["judged"]["typ"]
            if typ is None:
                continue
            win, loss = win + typ[0], loss + typ[2]
            health += typ[3]
            turns += 1
            continue
        fights = [rep["judged"]["typ"] for rep in record["players"].get(player, [])
                  if rep["judged"]["typ"] is not None]
        if not fights:
            continue
        win += mean([f[0] for f in fights])
        loss += mean([f[2] for f in fights])
        health += mean([f[3] for f in fights])
        turns += 1
    return win, loss, health, turns


def summary_row(records, player) -> dict:
    """The numbers a table line reports for one set of turns."""
    win, tie, loss, health, turns = pooled_opp(records, player)
    era_win, era_tie, era_loss, era_health, era_turns = pooled_opp(records, player, era=True)
    typ_win, typ_loss, typ_health, typ_turns = pooled_typ(records, player)
    era_typ_win, era_typ_loss, era_typ_health, _ = pooled_typ(records, player, era=True)
    bot_rate = win / max(1e-9, win + tie + loss)
    era_rate = era_win / max(1e-9, era_win + era_tie + era_loss)
    typ_rate = typ_win / max(1e-9, typ_win + typ_loss)
    era_typ_rate = era_typ_win / max(1e-9, era_typ_win + era_typ_loss)
    reps = [rep for r in records for rep in r["players"].get(player, [])]
    all_path = [rep["left_at"] < 0 for rep in reps]
    return {
        "turns": len(records),
        "opp_turns": turns,
        "opp": bot_rate,
        "era_opp": era_rate,
        "ratio": bot_rate / era_rate if era_rate > 0 else None,
        "opp_hp": health / max(1, turns),
        "era_opp_hp": era_health / max(1, era_turns),
        "typ": typ_rate,
        "era_typ": era_typ_rate,
        "typ_ratio": typ_rate / era_typ_rate if era_typ_rate > 0 else None,
        "typ_hp": typ_health / max(1, typ_turns),
        "era_typ_hp": era_typ_health / max(1, typ_turns),
        "on_path": mean([1.0 if p else 0.0 for p in all_path]),
        "value": mean([rep["value"] for rep in reps]),
        "era_value": mean([r["era"]["judged"]["value"] for r in records]),
        "gold": mean([rep["gold"] for rep in reps]),
        "era_gold": mean([r["era"]["gold"] for r in records]),
        "cycles": mean([len(rep["cycles"]) for rep in reps]),
        "era_cycles": mean([len(r["era"]["cycles"]) for r in records]),
        "tier": mean([rep["tier"] for rep in reps]),
        "era_tier": mean([r["era"]["tier"] for r in records]),
        "levelled": mean([1.0 if rep["levelled"] else 0.0 for rep in reps]),
        "era_levelled": mean([1.0 if r["era"]["levelled"] else 0.0 for r in records]),
        "steps": mean([rep["steps_on_path"] for rep in reps]),
        "era_steps": mean([r["logged_steps"] for r in records]),
    }


HEAD = ("turns", "opp", "era_opp", "ratio", "dHP", "typ", "era_typ", "typ_ratio",
        "dHP_typ", "on_path", "value", "era_value", "gold", "cycles", "tier")


def table(group_key: str | None, records, players) -> str:
    """One markdown table: per player overall (`group_key` None) or per group."""
    groups = defaultdict(list)
    if group_key is None:
        groups["all"] = records
    else:
        for record in records:
            groups[record[group_key]].append(record)
    lines = []
    head = f"| {group_key or 'group'} | player | " + " | ".join(HEAD) + " |"
    lines.append(head)
    lines.append("|" + "---|" * (len(HEAD) + 2))
    for group in sorted(groups):
        for player in players:
            rows = turns_with(groups[group], player)
            if not rows:
                continue
            s = summary_row(rows, player)
            cells = [
                str(s["turns"]),
                fmt(s["opp"], 3, 100, "%"),
                fmt(s["era_opp"], 3, 100, "%"),
                fmt(s["ratio"], 2, 100, "%"),
                fmt(None if s["opp_hp"] is None or s["era_opp_hp"] is None
                    else s["opp_hp"] - s["era_opp_hp"], 2),
                fmt(s["typ"], 3, 100, "%"),
                fmt(s["era_typ"], 3, 100, "%"),
                fmt(s["typ_ratio"], 2, 100, "%"),
                fmt(None if s["typ_hp"] is None or s["era_typ_hp"] is None
                    else s["typ_hp"] - s["era_typ_hp"], 2),
                fmt(s["on_path"], 2, 100, "%"),
                fmt(s["value"], 3),
                fmt(s["era_value"], 3),
                fmt(s["gold"], 2),
                fmt(s["cycles"], 2),
                fmt(s["tier"], 2),
            ]
            lines.append(f"| {group} | {player} | " + " | ".join(cells) + " |")
    return "\n".join(lines)


def turn_rates(record, player):
    """One turn's win rate against Era's real next opponent for the bot (the
    mean over repeats) and for Era's own replayed position, or None."""
    if not record["opponent_found"]:
        return None
    era = record["era"]["judged"]["opp"]
    if era is None:
        return None
    opps = [rep["judged"]["opp"] for rep in record["players"].get(player, [])
            if rep["judged"]["opp"] is not None]
    if not opps:
        return None
    bot = mean([o[0] / max(1e-9, o[0] + o[1] + o[2]) for o in opps])
    return bot, era[0] / max(1e-9, sum(era[:3]))


def bootstrap(records, players, n=2000, seed=7) -> list[str]:
    """The main number's uncertainty: the turns are the sample, so resample
    them. Per player, the 95% interval of the win rate against Era's real next
    opponent and of the ratio (both rates recomputed on the same draw); and the
    paired difference between two players (the first minus the second)."""
    rng = np.random.default_rng(seed)
    rates = {}
    for player in players:
        rows = [r for r in (turn_rates(record, player) for record in records) if r]
        rates[player] = np.array(rows) if rows else np.zeros((0, 2))
    lines = []
    for player, arr in rates.items():
        if len(arr) < 5:
            continue
        bot = arr[:, 0].mean()
        era = arr[:, 1].mean()
        idx = rng.integers(0, len(arr), (n, len(arr)))
        bots = arr[idx, 0].mean(axis=1)
        eras = arr[idx, 1].mean(axis=1)
        ratio = bots / np.where(eras > 0, eras, np.nan)
        lo, hi = np.nanpercentile(bots, [2.5, 97.5])
        rlo, rhi = np.nanpercentile(ratio, [2.5, 97.5])
        lines.append(
            f"- **{player}**: win rate against Era's real next opponent "
            f"{bot * 100:.1f}% (95% {lo * 100:.1f} to {hi * 100:.1f}), "
            f"ratio to Era {100 * bot / era:.0f}% "
            f"(95% {100 * rlo:.0f} to {100 * rhi:.0f}), over {len(arr)} turns."
        )
    if len(players) >= 2:
        a, b = rates[players[0]], rates[players[1]]
        if len(a) == len(b) and len(a) >= 5:
            d = a[:, 0] - b[:, 0]
            idx = rng.integers(0, len(d), (n, len(d)))
            ds = d[idx].mean(axis=1)
            lo, hi = np.percentile(ds, [2.5, 97.5])
            lines.append(
                f"- **{players[0]} minus {players[1]}**: {d.mean() * 100:+.1f} points of "
                f"win rate per turn (95% {lo * 100:+.1f} to {hi * 100:+.1f}); "
                f"{players[0]} wins more turns in {int((d > 0).sum())}, "
                f"{players[1]} in {int((d < 0).sum())}, {int((d == 0).sum())} tie."
            )
    return lines


def per_turn_score(record, player):
    """The main number for one turn: the bot's win rate against Era's real next
    opponent minus Era's own. None when there is no opponent board."""
    if not record["opponent_found"]:
        return None
    opps = [rep["judged"]["opp"] for rep in record["players"].get(player, [])
            if rep["judged"]["opp"] is not None]
    era_opp = record["era"]["judged"]["opp"]
    if not opps or era_opp is None:
        return None
    win = mean([o[0] / max(1e-9, o[0] + o[1] + o[2]) for o in opps])
    era_win = era_opp[0] / max(1e-9, era_opp[0] + era_opp[1] + era_opp[2])
    return win - era_win, win, era_win


def extreme(records, player, best: bool, n: int = 10) -> str:
    scored = []
    for record in records:
        s = per_turn_score(record, player)
        if s is not None:
            scored.append((s[0], record, s[1], s[2]))
    scored.sort(key=lambda row: row[0], reverse=best)
    lines = []
    for delta, record, win, era_win in scored[:n]:
        opps = [rep["judged"]["opp"] for rep in record["players"][player]
                if rep["judged"]["opp"] is not None]
        hp = mean([o[3] for o in opps])
        era_hp = record["era"]["judged"]["opp"][3]
        lines.append(
            f"\n**{record['file']} turn {record['turn']}** ({record['comp']}, "
            f"{record['bucket']}): bot wins {win * 100:.0f}% vs Era {era_win * 100:.0f}%, "
            f"Health {hp:+.2f} vs Era {era_hp:+.2f}, "
            f"left Era's path at step {record['players'][player][0]['left_at']} "
            f"of {record['logged_steps']}\n"
            f"\n- bot: {record['players'][player][0]['board']}\n"
            f"- Era: {record['era']['board']}\n"
        )
    return "\n".join(lines)


def fidelity(records, args) -> str:
    """The replay fidelity: the era player's end-of-turn position against the
    log's own, whole state and board."""
    turns = len(records)
    first = sum(1 for r in records if r["era"]["match_first"])
    any_seed = sum(1 for r in records if r["era"]["match_any"])
    board_ok = sum(
        1 for r in records
        if not any(d.startswith("board") for d in r["era"]["diffs"])
    )
    kinds = Counter()
    for record in records:
        for diff in record["era"]["diffs"]:
            kinds[diff.split(":")[0]] += 1
    lines = [
        f"{turns} turns: the era player's end-of-turn position is exactly the log's "
        f"in {first} turns at the first seed ({100 * first / max(1, turns):.1f}%), "
        f"and in {any_seed} turns with any of the {args.fidelity_seeds} seeds tried "
        f"({100 * any_seed / max(1, turns):.1f}%); its board is the log's "
        f"(no board difference) in {board_ok} ({100 * board_ok / max(1, turns):.1f}%).",
        "",
        "The state before every step is seated from the log by construction (that "
        "is what Era's luck means), so this measures the loader's last step and the "
        "end-of-turn effects. The shop replay's own per-step report "
        "(`bgbench shopreplay`) measures every step.",
    ]
    if kinds:
        lines.append("")
        lines.append("What differs, by field: " + ", ".join(
            f"{k} {v}" for k, v in kinds.most_common(10)))
    return "\n".join(lines)


def logged_results(directory: Path) -> dict:
    """What the log says happened to Era in each combat, by (source, game,
    turn): the `result` on her own board in `boards/real`."""
    out = {}
    for path in sorted(directory.glob("*.json")):
        if path.name.startswith("."):
            continue
        try:
            raw = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        for board in raw.get("boards", []):
            if board.get("who") == "me" and board.get("result"):
                out[(raw.get("source"), raw.get("game"), int(board.get("turn", 0)))] = board["result"]
    return out


def fight_check(records, directory: Path) -> str:
    """Era's replayed board against the board she really fought, split by what
    the log says the real combat did. The engine's fight is not the real one,
    so the split is not 100%; a check that the opponent board and the fight's
    direction are right."""
    results = logged_results(directory)
    by = defaultdict(list)
    for record in records:
        happened = results.get((record["source"], record["game"], record["turn"]))
        era = record["era"]["judged"]["opp"]
        if happened is None or era is None:
            continue
        by[happened].append(era[0] / max(1e-9, sum(era[:3])))
    if not by:
        return "The log's own results were not found for these turns."
    lines = [
        "Era's replayed board, fighting the board she really met, split by what "
        "the log says the real combat did (the engine's fight is not the real "
        "one, so the split is not 100%; this checks the opponent board and the "
        "fight's direction):",
        "",
    ]
    for name, label in (("win", "won"), ("tie", "tied"), ("lose", "lost")):
        if name in by:
            lines.append(
                f"- Era {label} the real combat in {len(by[name])} turns: the "
                f"engine gives her board {100 * mean(by[name]):.1f}% wins there."
            )
    return "\n".join(lines)


def skip_kinds(skipped) -> Counter:
    """Why the skipped turns are skipped, in groups: no number can be measured
    for them."""
    kinds = Counter()
    for entry in skipped:
        why = entry["why"]
        if "the file does not load" in why:
            kinds["turn(s) in a file that does not load"] += 1
        elif "Era's own line cannot be replayed" in why:
            kinds["Era's own line cannot be replayed"] += 1
        elif "cannot be seated" in why:
            kinds["turn(s) the loader cannot seat"] += 1
        else:
            kinds["turn(s) with another failure"] += 1
    return kinds


def write_report(records, skipped, bad, args, out: Path, run_seconds: float) -> None:
    players = [p for p in args.players]
    lines = [
        f"# The bot plays Era's turns, with Era's luck ({args.name})",
        "",
        f"- Network: `{args.ckpt}`",
        f"- Turns: {len(records)} measured of {len(records) + len(skipped)} from turn "
        f"{args.from_turn} on ({len(skipped)} skipped, see `skipped.tsv`: "
        + ", ".join(f"{v} {k}" for k, v in skip_kinds(skipped).most_common()) + ")",
        f"- Players: {', '.join(players)}; repeats: {args.repeats}; "
        f"fights per opponent: {args.fights}; typical boards: {args.typical}",
        (f"- Sample: {args.sample} turns spread evenly over the seatable turns"
         if args.sample else "- Sample: all seatable turns"),
        f"- Search: fight weight {args.fight_weight:g}, {args.playouts} playouts, "
        f"playout temperature {args.playout_temperature:g}, at most {args.playout_actions} "
        f"playout actions, margin {args.margin:g}, {args.search_fights} fights, "
        f"replan {args.replan}",
        f"- Run time: {run_seconds / 60:.1f} min of {args.dev}",
        f"- Action files that do not load: {len(bad)}"
        + ("" if not bad else " (" + "; ".join(bad) + ")"),
        "",
        "## Replay fidelity",
        "",
        fidelity(records, args),
        "",
        "## The fights against Era's real opponents",
        "",
        fight_check(records, Path(args.real_boards)),
        "",
        "## Overall",
        "",
        table(None, records, players),
        "",
        "`opp` is the win rate against the board Era really fought after the turn, "
        "`typ` against 20 real boards of that turn; `ratio` is the bot's rate over "
        "Era's own (100% = as strong as Era's board); `dHP` is the bot's average "
        "Health change minus Era's (per fight; + is better), and `on path` the share "
        "of repeats that played exactly Era's steps. "
        "`value` is the network's value of the end position. A `ratio` over 100% "
        "can mean the bot is stronger or that Era's own rate on those turns was "
        "near zero: read it with `era_opp` beside it.",
        "",
    ]
    era_gold = [r["era"]["gold"] for r in records]
    lines.append(
        f"Era ends a turn with 3 or more Gold in "
        f"{100 * mean([1.0 if g >= 3 else 0.0 for g in era_gold]):.0f}% of turns."
    )
    for player in players:
        reps = [rep for r in records for rep in r["players"].get(player, [])]
        if reps:
            lines.append(
                f"{player} ends one with 3 or more Gold in "
                f"{100 * mean([1.0 if rep['gold'] >= 3 else 0.0 for rep in reps]):.0f}% "
                f"of repeats."
            )
    lines += [
        "",
        "## Uncertainty",
        "",
        "Bootstrap over turns (2000 draws, the turns resampled with replacement); "
        "a turn is the unit because both players play the same turns.",
        "",
        *bootstrap(records, players),
        "",
        "## Per comp",
        "",
        table("comp", records, players),
        "",
        "## Per comp family",
        "",
        table("family", records, players),
        "",
        "## Per turn bucket",
        "",
        table("bucket", records, players),
        "",
        "## How often the bot stays on Era's path",
        "",
    ]
    errors = [(r["file"], r["turn"], rep["error"])
              for r in records for player in players
              for rep in r["players"].get(player, []) if rep.get("error")]
    if errors:
        lines.append(f"- **{len(errors)} repeats hit an engine refusal** "
                     f"(the first: {errors[0][0]} turn {errors[0][1]}: "
                     f"{errors[0][2]}); see the `error` column in `turns.tsv`.")
        lines.append("")
    for player in players:
        reps = [rep for r in records for rep in r["players"].get(player, [])]
        if not reps:
            continue
        whole = sum(1 for rep in reps if rep["left_at"] < 0)
        left_at = Counter(rep["left_at"] for rep in reps if rep["left_at"] >= 0)
        kinds = Counter()
        for rep in reps:
            why = rep["left_why"]
            if "Era played" in why:
                kinds["a different action at Era's step"] += 1
            elif "cannot be seated" in why:
                kinds["the log's next step could not be seated"] += 1
            elif "logged steps were over" in why:
                kinds["Era's logged steps were over"] += 1
            else:
                kinds["other"] += 1
        took1 = sum(1 for rep in reps if rep["steps_on_path"] >= 1)
        took3 = sum(1 for rep in reps if rep["steps_on_path"] >= 3)
        lines.append(
            f"- **{player}**: {whole} of {len(reps)} repeats ({100 * whole / len(reps):.1f}%) "
            f"played every one of Era's steps; at least her first step in {took1} "
            f"({100 * took1 / len(reps):.1f}%), at least three in {took3} "
            f"({100 * took3 / len(reps):.1f}%); "
            f"{mean([rep['steps_on_path'] for rep in reps]):.1f} of "
            f"{mean([rep['logged_steps'] for rep in reps]):.1f} logged steps taken on "
            f"average. Where it left: "
            + ", ".join(f"step {k} {v}" for k, v in sorted(left_at.items())[:12])
            + (f" (and {len(left_at) - 12} later steps)" if len(left_at) > 12 else "")
        )
        if kinds:
            lines.append(f"  - why: " + ", ".join(f"{k} {v}" for k, v in kinds.most_common(4)))
    lines += ["", "## Furthest behind Era (10 turns)", ""]
    for player in players:
        lines.append(f"### {player}")
        lines.append(extreme(records, player, best=False))
    lines += ["", "## Furthest ahead of Era (10 turns)", ""]
    for player in players:
        lines.append(f"### {player}")
        lines.append(extreme(records, player, best=True))
    lines += [
        "",
        "## Two things read off the turns",
        "",
        "- The search's own choice does not go through `ppo.force_mask`: the "
        "Searcher forces the network's actions (the plain baseline and the "
        "actions inside its playouts), not the candidates it compares, so on "
        "turns 3 to 5 it can skip the Dark Discovery the plain player is forced "
        "to press. Part of its gap on those turns can be that, not strength. This "
        "is search v1 as committed, not changed here.",
        "- The search often ends a turn with Gold in hand (see the 3+ Gold share "
        "above): its candidate list holds End Turn, and the network's value plus "
        "the fight judge can prefer that to spending. Era almost never ends a "
        "turn with 3 or more Gold.",
        "",
        "## What cannot be measured",
        "",
        "- The turns in `skipped.tsv` get no numbers: the loader cannot seat the "
        "turn (a trinket pick whose offer holds a trinket the trinket table does not "
        "build is the common one; turns 6 and 9 are almost all of those, because "
        "that is where the trinket offers open), the action file does not load, or "
        "Era's own line cannot be replayed (an unbuilt hero power on one of her "
        "steps, so there is no Era position to compare against).",
        f"- {sum(1 for r in records if r['opponent_found'])} of {len(records)} turns "
        "found the board Era really fought next; the rest fought a ghost or have no "
        "opponent board in `boards/real`, so their headline numbers are blank.",
    ]
    (out / "report.md").write_text("\n".join(lines) + "\n")


def write_tsv(records, args, out: Path) -> None:
    cols = [
        "file", "turn", "comp", "family", "bucket", "player",
        "logged_steps", "steps_on_path", "left_at", "on_path",
        "opp_w", "opp_t", "opp_l", "opp_hp",
        "era_opp_w", "era_opp_t", "era_opp_l", "era_opp_hp",
        "typ_w", "typ_l", "typ_hp", "era_typ_w", "era_typ_l", "era_typ_hp",
        "value", "era_value", "gold", "era_gold", "cycles", "era_cycles",
        "tier", "era_tier", "levelled", "era_levelled", "error",
    ]
    lines = ["\t".join(cols)]
    for record in sorted(records, key=lambda r: (r["file"], r["turn"])):
        for player in args.players:
            reps = record["players"].get(player, [])
            if not reps:
                continue
            opp = [rep["judged"]["opp"] for rep in reps]
            typ = [rep["judged"]["typ"] for rep in reps]
            era_opp = record["era"]["judged"]["opp"]
            era_typ = record["era"]["judged"]["typ"]
            cells = [
                record["file"], str(record["turn"]), record["comp"], record["family"],
                record["bucket"], player,
                str(record["logged_steps"]),
                fmt(mean([rep["steps_on_path"] for rep in reps]), 1),
                "|".join(str(rep["left_at"]) for rep in reps),
                fmt(mean([1.0 if rep["left_at"] < 0 else 0.0 for rep in reps]), 2),
                fmt(mean([o[0] for o in opp if o is not None]), 2),
                fmt(mean([o[1] for o in opp if o is not None]), 2),
                fmt(mean([o[2] for o in opp if o is not None]), 2),
                fmt(mean([o[3] for o in opp if o is not None]), 2),
                fmt(era_opp[0] if era_opp else None, 2),
                fmt(era_opp[1] if era_opp else None, 2),
                fmt(era_opp[2] if era_opp else None, 2),
                fmt(era_opp[3] if era_opp else None, 2),
                fmt(mean([t[0] for t in typ if t is not None]), 2),
                fmt(mean([t[2] for t in typ if t is not None]), 2),
                fmt(mean([t[3] for t in typ if t is not None]), 2),
                fmt(era_typ[0] if era_typ else None, 2),
                fmt(era_typ[2] if era_typ else None, 2),
                fmt(era_typ[3] if era_typ else None, 2),
                fmt(mean([rep["value"] for rep in reps]), 4),
                fmt(record["era"]["judged"]["value"], 4),
                fmt(mean([rep["gold"] for rep in reps]), 1),
                fmt(record["era"]["gold"], 1),
                fmt(mean([len(rep["cycles"]) for rep in reps]), 2),
                fmt(len(record["era"]["cycles"]), 2),
                fmt(mean([rep["tier"] for rep in reps]), 1),
                fmt(record["era"]["tier"], 1),
                fmt(mean([1.0 if rep["levelled"] else 0.0 for rep in reps]), 2),
                fmt(1.0 if record["era"]["levelled"] else 0.0, 2),
                "|".join("" if r.get("error") is None else r["error"].replace("\t", " ")
                         for r in reps),
            ]
            lines.append("\t".join(cells))
    (out / "turns.tsv").write_text("\n".join(lines) + "\n")


# ---- main ----------------------------------------------------------------


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--name", default="plain-search", help="the run's name (the output folder)")
    ap.add_argument("--ckpt", default=str(DEFAULT_CKPT), help="the network")
    ap.add_argument("--players", default="plain,search",
                    help="comma list of plain, search and planner")
    ap.add_argument("--planner", default="",
                    help="the comp library TOML; adds the planner player")
    ap.add_argument("--from-turn", type=int, default=3, help="the first logged turn to play")
    ap.add_argument("--repeats", type=int, default=3, help="repeats per turn and player")
    ap.add_argument("--fights", type=int, default=100,
                    help="fights against Era's real next opponent")
    ap.add_argument("--typical", type=int, default=20,
                    help="real boards of the turn the position fights")
    ap.add_argument("--fidelity-seeds", type=int, default=8,
                    help="seeds tried for an Era position that is exactly the log's")
    ap.add_argument("--max-actions", type=int, default=200,
                    help="the most actions one turn may take")
    ap.add_argument("--turns", type=int, default=0, help="play only the first N turns")
    ap.add_argument("--sample", type=int, default=0,
                    help="play N turns spread evenly over the seatable turns (0: all)")
    ap.add_argument("--seed", type=int, default=11)
    ap.add_argument("--playouts", type=int, default=1,
                    help="the search's playouts (greedy playouts are deterministic, 1 is enough)")
    ap.add_argument("--playout-temperature", type=float, default=0.0,
                    help="the network's temperature after a candidate's first move (0 = greedy)")
    ap.add_argument("--playout-actions", type=int, default=60,
                    help="the most shop actions one playout takes")
    ap.add_argument("--margin", type=float, default=0.0,
                    help="a candidate must beat the network's own choice by this value margin")
    ap.add_argument("--search-fights", type=int, default=20,
                    help="real boards per search playout")
    ap.add_argument("--fight-weight", type=float, default=1.0,
                    help="the search's fight weight (search v1's best: 1)")
    ap.add_argument("--replan", choices=["every", "turn", "cycle"], default="every")
    ap.add_argument("--real-boards", default=str(ROOT / "boards" / "real"))
    ap.add_argument("--actions",
                    default=str(ROOT / "boards" / "frozen" / "actions-2026-11-15"))
    ap.add_argument("--out", default=None, help="the output folder")
    ap.add_argument("--vram-gb", type=float, default=0.0,
                    help="cap this process at this much GPU memory (6 when pcbusy says ON)")
    ap.add_argument("--old-boards", choices=["keep", "drop"], default="keep",
                    help="what an opponent's last-seen board is worth (ppo.py's)")
    ap.add_argument("--force-dark-gift", action=argparse.BooleanOptionalAction, default=True,
                    help="press Dark Discovery on turns 3 to 5 (the r1106 run trained "
                         "with it, and its evals force it: ppo.force_mask)")
    ap.add_argument("--force-turn2-level", action="store_true",
                    help="ppo.py's --force-turn2-level, for a run trained with it")
    ap.add_argument("--resume", action=argparse.BooleanOptionalAction, default=True,
                    help="pick up the turns already in turns.jsonl")
    ap.add_argument("--run-seconds", type=float, default=0.0,
                    help="the run time to report (0: measure this process); a rerun "
                         "that only rewrites the report keeps the first run's")
    args = ap.parse_args()
    if args.turns and args.sample:
        raise SystemExit("--turns and --sample are exclusive: --sample spreads N, --turns takes the first N")

    args.players = [p.strip() for p in args.players.split(",") if p.strip()]
    if args.planner and "planner" not in args.players:
        args.players.append("planner")
    for player in args.players:
        if player not in ("plain", "search", "planner"):
            raise SystemExit(f"no player {player}: plain, search or planner")
        if player == "planner" and not args.planner:
            raise SystemExit("--players planner needs --planner PATH")
    args.dev = "cuda" if torch.cuda.is_available() else "cpu"
    if args.dev == "cuda" and args.vram_gb > 0:
        torch.cuda.set_per_process_memory_fraction(args.vram_gb / 16.3)
        print(f"GPU capped at {args.vram_gb} GB")
    ppo.FORCE_DARK = args.force_dark_gift
    ppo.FORCE_TURN2 = args.force_turn2_level
    ppo.OLD_BOARDS = args.old_boards

    out = Path(args.out) if args.out else ROOT / "train" / "runs" / f"era-turns-{args.name}"
    out.mkdir(parents=True, exist_ok=True)
    net = load(args.ckpt, args.dev)
    net.eval()
    print(f"{args.ckpt}: {args.dev}, output {out}", flush=True)

    specs, unknown = real_boards.load(Path(args.real_boards))
    print(f"real boards: {len(specs)} ({len(unknown)} unknown minions)", flush=True)
    replays, bad = bgpy.era_replays(args.actions, str(ROOT / "ref" / "bg-cards.json"), specs)
    fights, unknown_fights = bgpy.RealFights.load(args.real_boards, str(ROOT / "ref" / "bg-cards.json"))
    print(f"real fight boards: {fights.len()} ({len(unknown_fights)} unknown minions)", flush=True)

    t_start = time.time()
    records, skipped = run(net, args.dev, args, replays, bad, fights, out, specs)
    run_seconds = args.run_seconds if args.run_seconds > 0 else time.time() - t_start
    lines = ["file\tturn\twhy"]
    lines += [f"{s['file']}\t{s['turn']}\t{s['why']}" for s in skipped]
    (out / "skipped.tsv").write_text("\n".join(lines) + "\n")
    write_tsv(records, args, out)
    write_report(records, skipped, bad, args, out, run_seconds)
    print(f"wrote {out}/turns.tsv and {out}/report.md in {run_seconds / 60:.1f} min", flush=True)


if __name__ == "__main__":
    main()
