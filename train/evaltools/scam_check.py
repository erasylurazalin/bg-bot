"""Measure one-card answers to being out-statted, for docs/briefs/scam-check.md.

Plays the fixed eval's games at lag 0 and at lag 2 from turn 7 (2,048 games
each, the same seeds, lineup, frozen boards and lobby filter as
train/fixed_eval.py). For every learner turn from 10 on it records, at the
moment the turn ends (End Turn or the action cap), the learner's board and the
board of the opponent the engine paired it with that round. It also records
whether Leeroy or a Venomous/Poisonous minion was offered in the Tavern during
that turn. The same measures are taken from Era's real boards.

The opponent pairing comes from the learner's own observation: the opponent
block whose OPP_NEXT bit is set, mapped to seat (learner + k) % PLAYERS. The
board is read from `env.seats` for a live player. A ghost's board is not
exposed to Python, so a ghost row uses the last-seen board in that opponent's
observation slots (and is marked ghost in rows.json).

    .venv/bin/python train/evaltools/scam_check.py --vram-gb 10

Writes train/runs/scam-check/rows.json and report.md. Measurement only.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

import numpy as np
import torch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "train"))

import bgpy
import fixed_eval
import ppo
import real_boards
from ppo import Bufs, F_COMP, LEARNER, comp_onehot, force_mask

CKPT = ROOT / "train" / "runs" / "r1111-lagmix" / "best.pt"
OUT = ROOT / "train" / "runs" / "scam-check"
LEEROY = "Leeroy the Reckless"
FIRST_TURN = 10
BANDS = (("10-12", 10, 12), ("13+", 13, 10 ** 6), ("all", 10, 10 ** 6))
# Keywords::ALL order in bgcore: taunt, divine_shield, windfury, venomous,
# reborn, magnetic, stealth, poisonous.
VENOMOUS = bgpy.SLOT_KEYWORDS + 3
POISONOUS = bgpy.SLOT_KEYWORDS + 7


def parse_args():
    a = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    a.add_argument("--ckpt", type=Path, default=CKPT)
    a.add_argument("--games", type=int, default=2048)
    a.add_argument("--lags", default="0,2",
                   help="comma-separated real-seat lags (default 0,2)")
    a.add_argument("--lag2-from", type=int, default=7,
                   help="first turn a nonzero lag applies on (default 7)")
    a.add_argument("--seed-base", type=int, default=fixed_eval.SEED_BASE)
    a.add_argument("--boards", type=Path, default=fixed_eval.FROZEN)
    a.add_argument("--vram-gb", type=float,
                   help="cap this process's GPU memory; required when CUDA is used")
    a.add_argument("--cpu", action="store_true", help="run on the CPU")
    a.add_argument("--out-dir", type=Path, default=OUT)
    return a.parse_args()


def keyword_set(text: str) -> set[str]:
    return {k for k in text.split(",") if k}


def minion_row(name, attack, health, keywords, golden):
    return {
        "name": name,
        "attack": int(attack),
        "health": int(health),
        "golden": bool(golden),
        "keywords": sorted(keyword_set(keywords)),
    }


def board_from_seat(seats, seat: int) -> list[dict]:
    # env.seats gives (name, golden, attack, health, keywords).
    return [minion_row(m[0], m[2], m[3], m[4], m[1]) for m in seats[seat][5]]


def board_from_slots(row, start: int, names: list[str]) -> list[dict]:
    """The board in an observation's slot block (used for a ghost)."""
    out = []
    for i in range(bgpy.BOARD):
        b = start + i * bgpy.SLOT_FEATURES
        if row[b + bgpy.SLOT_PRESENT] <= 0.5:
            continue
        kw = []
        for j in range(8):
            if row[b + bgpy.SLOT_KEYWORDS + j] > 0.5:
                kw.append(("taunt", "divine_shield", "windfury", "venomous",
                           "reborn", "magnetic", "stealth", "poisonous")[j])
        out.append(minion_row(
            names[int(row[b + bgpy.SLOT_CARD_ID]) - 1],
            row[b + bgpy.SLOT_ATTACK], row[b + bgpy.SLOT_HEALTH],
            ",".join(kw), row[b + bgpy.SLOT_GOLDEN] > 0.5))
    return out


def shop_minions(row, names: list[str]) -> list[dict]:
    """The Tavern's minion slots, with the engine's own keyword flags."""
    out = []
    for i in range(bgpy.SHOP):
        b = bgpy.SHOP_START + i * bgpy.SLOT_FEATURES
        if row[b + bgpy.SLOT_PRESENT] <= 0.5 or row[b + bgpy.SLOT_IS_SPELL] > 0.5:
            continue
        out.append({
            "name": names[int(row[b + bgpy.SLOT_CARD_ID]) - 1],
            "venomous": row[b + VENOMOUS] > 0.5,
            "poisonous": row[b + POISONOUS] > 0.5,
        })
    return out


def holds(board: list[dict]) -> dict:
    return {
        "leeroy": any(m["name"] == LEEROY for m in board),
        "venom": any(("venomous" in m["keywords"])
                     or ("poisonous" in m["keywords"]) for m in board),
        "either": any(m["name"] == LEEROY or "venomous" in m["keywords"]
                      or "poisonous" in m["keywords"] for m in board),
    }


def total_stats(board: list[dict]) -> int:
    return sum(m["attack"] + m["health"] for m in board)


def opponent_block(row, learner_seat: int):
    """(seat, block start, alive) for the opponent this learner fights."""
    for k in range(1, bgpy.PLAYERS):
        block = bgpy.OWN_END + (k - 1) * bgpy.OPP_BLOCK
        if row[block + bgpy.OPP_NEXT] > 0.5:
            seat = (learner_seat + k) % bgpy.PLAYERS
            return seat, block, row[block + bgpy.OPP_ALIVE] > 0.5
    return None, None, False


def band_of(turn: int) -> str | None:
    for label, lo, hi in BANDS:
        if lo <= turn <= hi:
            return label
    return None


def make_bot_row(game, lag, turn, action, actions_left, seats, row, names,
                 learner_seat, offer) -> dict:
    board = board_from_seat(seats, learner_seat)
    opp_seat, block, opp_alive = opponent_block(row, learner_seat)
    ghost = row[bgpy.SCALAR_GHOST] > 0.5
    if opp_seat is None:
        opp_board = []
        opp_source = "none"
    elif ghost:
        # A ghost's board lives only inside the engine. The observation's
        # opponent slots carry the last board we saw of that player.
        opp_board = board_from_slots(row, block + bgpy.OPP_SCALARS, names)
        opp_source = "observation_last_seen"
    else:
        opp_board = board_from_seat(seats, opp_seat)
        opp_source = "env.seats"
    mine_total = total_stats(board)
    opp_total = total_stats(opp_board)
    hold = holds(board)
    return {
        "group": f"bot lag {lag}",
        "game": int(game),
        "lag": int(lag),
        "turn": int(turn),
        "band": band_of(turn),
        "learner_seat": int(learner_seat),
        "opp_seat": None if opp_seat is None else int(opp_seat),
        "opp_alive": bool(opp_alive),
        "opp_ghost": bool(ghost),
        "opp_board_source": opp_source,
        "ended_by": "end_turn" if int(action) == ppo.END_TURN else "cap",
        "actions_left": int(actions_left),
        "board": board,
        "opp_board": opp_board,
        "total": mine_total,
        "opp_total": opp_total,
        "behind": 2 * mine_total < opp_total,
        "holds_leeroy": hold["leeroy"],
        "holds_venom": hold["venom"],
        "holds_either": hold["either"],
        "offered_leeroy": bool(offer[0]),
        "offered_venom": bool(offer[1]),
        "offered_either": bool(offer[2]),
        "offered_names": sorted(offer[3]),
    }


def collect_bot(args, info, real, lag: int, lag_from: int, dev) -> list[dict]:
    net = info["net"]
    rows: list[dict] = []
    names = None
    started = time.time()
    chunk = 0
    while chunk * fixed_eval.CHUNK < args.games:
        start = chunk * fixed_eval.CHUNK
        n = min(fixed_eval.CHUNK, args.games - start)
        env = bgpy.VecEnv(n, args.seed_base + chunk, fixed_eval.LINEUP,
                          real_boards=real, old_boards=info["old_boards"],
                          real_lobby_filter=True, real_lag=lag,
                          real_lag_from=lag_from)
        if info["overlay_comps"]:
            env.set_overlay(info["overlay_comps"], 0.0)
        if names is None:
            names = env.card_names()
        b = Bufs(n)
        env.kinds(b.kinds)
        done = np.zeros(n, dtype=bool)
        rec = [None] * n
        turn_rows: list[list[dict]] = [[] for _ in range(n)]
        # Offers seen during the turn being played, by env.
        offer_turn = [0] * n
        offer = [[False, False, False, set()] for _ in range(n)]
        ar = np.arange(n)
        torch.manual_seed(args.seed_base + chunk)
        steps = 0
        while not done.all():
            env.observe(b.obs, b.mask, b.seat, b.kind)
            turns = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
            left = b.obs[:, bgpy.SCALAR_ACTIONS_LEFT].astype(np.int64)
            current = (~done) & (b.kind == LEARNER)
            for e in np.nonzero(current)[0]:
                e = int(e)
                turn = int(turns[e])
                if turn < FIRST_TURN:
                    offer_turn[e] = turn
                    offer[e] = [False, False, False, set()]
                    continue
                if offer_turn[e] != turn:
                    offer_turn[e] = turn
                    offer[e] = [False, False, False, set()]
                for m in shop_minions(b.obs[e], names):
                    name = m["name"]
                    venom = m["venomous"] or m["poisonous"]
                    if name == LEEROY:
                        offer[e][0] = True
                    if venom:
                        offer[e][1] = True
                    if name == LEEROY or venom:
                        offer[e][2] = True
                        offer[e][3].add(name)
            o = torch.from_numpy(b.obs).to(dev)
            mask = force_mask(o, torch.from_numpy(b.mask).to(dev))
            comp = None
            if net.n_comps:
                env.overlay_probe(b.forced1, b.flags)
                comp_idx = b.flags[ar, b.seat, F_COMP] - 1
                comp = comp_onehot(comp_idx, net.n_comps, dev)
            acts = fixed_eval.sample_actions(net, o, mask, comp, 1.0)
            acts_np = acts.cpu().numpy()
            ends = (acts_np == ppo.END_TURN) | (left <= 1)
            for e in np.nonzero(current & ends)[0]:
                e = int(e)
                turn = int(turns[e])
                if turn < FIRST_TURN:
                    continue
                seats = env.seats(e)
                turn_rows[e].append(make_bot_row(
                    start + e, lag, turn, acts_np[e], left[e], seats,
                    b.obs[e], names, int(b.seat[e]), offer[e]))

            kinds = b.kinds.copy()
            env.step(acts_np, b.places, b.tier_turns, b.tribes, b.reset)
            for e, s in zip(*np.nonzero(b.places)):
                e, s = int(e), int(s)
                if kinds[e, s] != LEARNER or done[e]:
                    continue
                place = int(b.places[e, s])
                rec[e] = {
                    "game": start + e,
                    "chunk": chunk,
                    "env": e,
                    "place": place,
                    "knocked_out_turn": None if place == 1 else int(turns[e]),
                    "turns": turn_rows[e],
                }
                done[e] = True
            if b.reset.any():
                env.kinds(b.kinds)
            steps += 1
            if steps % 2000 == 0 and not done.all():
                print(f"    lag {lag} chunk {chunk}: {int(done.sum())}/{n} done, "
                      f"{time.time() - started:.0f}s", flush=True)
        for game in rec:
            if game is None:
                continue
            for row in game["turns"]:
                row["place"] = game["place"]
                row["knocked_out_turn"] = game["knocked_out_turn"]
                rows.append(row)
        print(f"  lag {lag} chunk {chunk} done: {int(done.sum())}/{n} games, "
              f"{len(rows)}/{args.games} total, {steps} steps, "
              f"{time.time() - started:.0f}s", flush=True)
        chunk += 1
    rows.sort(key=lambda r: (r["game"], r["turn"]))
    return rows


def collect_real(folder: Path) -> list[dict]:
    rows = []
    for path in sorted(folder.glob("*.json")):
        data = json.loads(path.read_text())
        by_turn: dict[int, dict[str, dict]] = {}
        for board in data["boards"]:
            by_turn.setdefault(int(board["turn"]), {})[board["who"]] = board

        def minions(board):
            return [{
                "name": m.get("name") or m.get("card_id") or "",
                "attack": int(m.get("attack") or 0),
                "health": int(m.get("health") or 0),
                "golden": bool(m.get("golden")),
                "keywords": sorted(k.lower() for k in (m.get("keywords") or [])),
            } for m in board.get("minions") or []]

        for turn in sorted(by_turn):
            if turn < FIRST_TURN:
                continue
            pair = by_turn[turn]
            for who, group, other in (("me", "era", "opp"),
                                      ("opp", "other", "me")):
                board = pair.get(who)
                if board is None:
                    continue
                opp = pair.get(other)
                mine = minions(board)
                theirs = minions(opp) if opp is not None else []
                mine_total = total_stats(mine)
                opp_total = total_stats(theirs)
                hold = holds(mine)
                rows.append({
                    "source": path.name,
                    "who": who,
                    "group": "Era" if group == "era" else "other players",
                    "turn": turn,
                    "band": band_of(turn),
                    "board": mine,
                    "opp_board": theirs,
                    "total": mine_total,
                    "opp_total": opp_total,
                    "behind": 2 * mine_total < opp_total,
                    "holds_leeroy": hold["leeroy"],
                    "holds_venom": hold["venom"],
                    "holds_either": hold["either"],
                })
    return rows


def pct(count: int, n: int) -> str:
    if n == 0:
        return "n/a"
    return f"{100 * count / n:.1f}% ({count}/{n})"


def table(rows: list[dict], groups, title: str) -> list[str]:
    lines = [f"## {title}", ""]
    lines.append("| group | band | boards | Leeroy | Venomous or Poisonous | either |")
    lines.append("|---|---|---|---|---|---|")
    for group in groups:
        for label, lo, hi in BANDS:
            sel = [r for r in rows if r["group"] == group
                   and lo <= r["turn"] <= hi]
            n = len(sel)
            h = holds_all(sel)
            lines.append(f"| {group} | {label} | {n} | {pct(h['leeroy'], n)} | "
                         f"{pct(h['venom'], n)} | {pct(h['either'], n)} |")
    lines.append("")
    return lines


def holds_all(rows: list[dict]) -> dict:
    return {
        "leeroy": sum(r["holds_leeroy"] for r in rows),
        "venom": sum(r["holds_venom"] for r in rows),
        "either": sum(r["holds_either"] for r in rows),
    }


def offer_table(bot_rows: list[dict]) -> list[str]:
    lines = ["## Bot Tavern offers on turns 10 and later", ""]
    lines.append("A turn counts as offered if any observation in that turn's "
                 "Tavern, before or after a Roll, held the card. Its Tavern "
                 "minions carry the engine's own keyword flags.")
    lines.append("")
    lines.append("| lag | band | turns | offered Leeroy | offered Venomous or "
                 "Poisonous | offered either |")
    lines.append("|---|---|---|---|---|---|")
    for lag in sorted({r["lag"] for r in bot_rows}):
        for label, lo, hi in BANDS:
            sel = [r for r in bot_rows if r["lag"] == lag and lo <= r["turn"] <= hi]
            n = len(sel)
            lines.append(
                f"| {lag} | {label} | {n} | "
                f"{pct(sum(r['offered_leeroy'] for r in sel), n)} | "
                f"{pct(sum(r['offered_venom'] for r in sel), n)} | "
                f"{pct(sum(r['offered_either'] for r in sel), n)} |")
    lines.append("")
    return lines


def offered_hold_table(bot_rows: list[dict], behind_only: bool = False) -> list[str]:
    if behind_only:
        title = "## Behind and shown in the Tavern"
        intro = ("Turns 10 and later with under half the opponent's stats, "
                 "ghost opponents left out. `held | offered` is among the "
                 "turns whose Tavern held that card; `held | not offered` is "
                 "among the turns whose Tavern did not.")
    else:
        title = "## Holding it when the Tavern showed it"
        intro = ("Turns 10 and later, all bands. `held | offered` is among the "
                 "turns whose Tavern held that card; `held | not offered` is "
                 "among the turns whose Tavern did not.")
    lines = [title, ""]
    lines.append(intro)
    lines.append("")
    lines.append("| lag | card | held when offered | held when not offered |")
    lines.append("|---|---|---|---|")
    keys = (("Leeroy", "offered_leeroy", "holds_leeroy"),
            ("Venomous or Poisonous", "offered_venom", "holds_venom"),
            ("either", "offered_either", "holds_either"))
    for lag in sorted({r["lag"] for r in bot_rows}):
        sel = [r for r in bot_rows if r["lag"] == lag]
        if behind_only:
            sel = [r for r in sel if not r.get("opp_ghost") and r["behind"]]
        for label, offered, held in keys:
            yes = [r for r in sel if r[offered]]
            no = [r for r in sel if not r[offered]]
            lines.append(
                f"| {lag} | {label} | "
                f"{pct(sum(r[held] for r in yes), len(yes))} | "
                f"{pct(sum(r[held] for r in no), len(no))} |")
    lines.append("")
    return lines


def share(rows: list[dict], key: str) -> str:
    return pct(sum(bool(r[key]) for r in rows), len(rows))


def group_rows(rows: list[dict], group: str) -> list[dict]:
    return [r for r in rows if r["group"] == group]


def short_answer(all_rows: list[dict], behind: list[dict],
                 bot_rows: list[dict], groups: list[str]) -> list[str]:
    lines = ["## Short answer", ""]
    parts = []
    for group in groups:
        rs = group_rows(behind, group)
        parts.append(f"{group} {share(rs, 'holds_either')}")
    lines.append("- Behind (under half the opponent's stats), holding either "
                 "Leeroy or a Venomous/Poisonous minion: " + "; ".join(parts) + ".")
    parts = []
    for group in groups:
        rs = group_rows(all_rows, group)
        parts.append(f"{group} {share(rs, 'holds_either')}")
    lines.append("- All turn 10+ boards: " + "; ".join(parts) + ".")
    parts = []
    for group in groups:
        rs = group_rows(behind, group)
        parts.append(f"{group} {share(rs, 'holds_leeroy')}")
    lines.append("- Leeroy on behind boards: " + "; ".join(parts) + ".")
    for lag in sorted({r["lag"] for r in bot_rows}):
        sel = [r for r in bot_rows if r["lag"] == lag]
        shown = [r for r in sel if r["offered_either"]]
        unseen = [r for r in sel if not r["offered_either"]]
        lines.append(f"- Lag {lag}: the Tavern showed either on "
                     f"{share(sel, 'offered_either')} of turns; the bot held "
                     f"either on {share(shown, 'holds_either')} of those and "
                     f"{share(unseen, 'holds_either')} of turns where it was "
                     "not shown.")
    parts = []
    for lag in sorted({r["lag"] for r in bot_rows}):
        sel = [r for r in bot_rows if r["lag"] == lag and r["behind"]
               and not r.get("opp_ghost")]
        shown = [r for r in sel if r["offered_either"]]
        unseen = [r for r in sel if not r["offered_either"]]
        parts.append(f"lag {lag} {share(shown, 'holds_either')} against "
                     f"{share(unseen, 'holds_either')}")
    lines.append("- On behind turns, held either when the Tavern showed either "
                 "against when it did not: " + "; ".join(parts) + ".")
    lines.append("")
    return lines


def build_report(payload: dict) -> str:
    bot = payload["bot_rows"]
    real = payload["real_rows"]
    all_rows = bot + real
    lines = ["# Scam check: one-card answers on turns 10 and later", ""]
    lines.append(f"- checkpoint: `{payload['checkpoint']}` (update {payload['update']})")
    lines.append(f"- games per lag: {payload['games']}, seeds {payload['seed_base']} "
                 f"onwards, chunk {payload['chunk']}, lineup {payload['lineup']}")
    lines.append(f"- frozen boards: `{payload['boards']}` "
                 f"({payload['n_board_files']} files, manifest checked)")
    lines.append(f"- lags: {payload['lags_text']}")
    lines.append("- a board is a turn-end board (the observation before the "
                 "last action: End Turn or the action cap)")
    lines.append("- behind: twice the board's Attack plus Health is under the "
                 "opponent's Attack plus Health")
    lines.append("- keywords are the engine's (`env.seats` for live opponents, "
                 "the observation's flags for Tavern slots)")
    lines.append(f"- git: {payload.get('git', 'unknown')}")
    lines.append(f"- written: {payload['written']}")
    lines.append("")
    lines.append("## How the opponent pairing was read")
    lines.append("")
    lines.append("The learner's observation has one block per opponent, in seat "
                 "order after the learner. `OPP_NEXT` is set from the engine's "
                 "`pl.opponent`, so the set bit gives the exact player (or the "
                 "dead player whose ghost is the opponent), and the block's "
                 "seat is `(learner seat + k) % 8`. Live opponents' boards are "
                 "read from `env.seats`; `SCALAR_GHOST` marks a ghost, whose "
                 "board is not exposed to Python, so the ghost's last-seen "
                 "board slots in the observation are used instead.")
    lines.append("")

    groups = ["bot lag 0", "bot lag 2", "Era", "other players"]
    known = [r for r in all_rows if not r.get("opp_ghost")]
    behind = [r for r in known if r["behind"]]
    rest = [r for r in known if not r["behind"]]
    lines.extend(short_answer(all_rows, behind, bot, groups))
    lines.extend(table(all_rows, groups, "All boards"))
    lines.extend(table(behind, groups,
                       "Boards with under half the opponent's Attack plus Health"))
    lines.extend(table(rest, groups,
                       "Boards not behind (at least half the opponent's stats)"))
    lines.extend(offer_table(bot))
    lines.extend(offered_hold_table(bot))
    lines.extend(offered_hold_table(bot, behind_only=True))

    ghost = sum(1 for r in bot if r["opp_ghost"])
    ghost_empty = sum(1 for r in bot if r["opp_ghost"] and not r["opp_board"])
    capped = sum(1 for r in bot if r["ended_by"] == "cap")
    lines.append("## Limits")
    lines.append("")
    lines.append(f"- Lag 2 has many more late boards than lag 0: "
                 f"{sum(1 for r in bot if r['lag'] == 2)} against "
                 f"{sum(1 for r in bot if r['lag'] == 0)} turn-end boards. The "
                 "lags are shown side by side and never pooled.")
    lines.append(f"- Ghost opponents: {ghost} of {len(bot)} bot boards "
                 f"({ghost_empty} with no last-seen board). A ghost's board is "
                 "not exposed to Python; the row carries the last board the "
                 "learner saw of that player, which can be stale. Ghost rows "
                 "are in the all-boards table but are left out of the "
                 "behind/not-behind split.")
    lines.append(f"- Action-cap endings: {capped} of {len(bot)} bot boards. "
                 "The board is the one the final action saw, before that action.")
    lines.append("- The pairing is read at the final action. A shop-phase "
                 "knockout in that same action would re-pair the survivors "
                 "before combat; this script does not detect that case, so "
                 "such a row would carry the old opponent.")
    lines.append("- Era and other players are the frozen real boards, one row "
                 "per game turn; a game that grouped several fights into one "
                 "logged turn is counted once.")
    lines.append("- The bot's Tavern offers cover only turns the learner played; "
                 "a turn cut short by death has no turn-end row.")
    lines.append("")
    return "\n".join(lines) + "\n"


def main():
    args = parse_args()
    if args.games < 1:
        raise SystemExit("--games must be at least 1")
    lags = [int(x) for x in args.lags.split(",") if x.strip()]
    folder = args.boards.resolve()
    n_board_files = fixed_eval.check_boards(folder)
    real, unknown = real_boards.load(folder)
    if not real:
        raise SystemExit(f"{folder}: no real boards")
    print(f"real boards from {folder}: {n_board_files} files, {len(real)} boards"
          + (f", {len(set(unknown))} unmatched names" if unknown else ""), flush=True)

    dev = torch.device("cpu" if args.cpu else
                       ("cuda" if torch.cuda.is_available() else "cpu"))
    if dev.type == "cuda":
        if args.vram_gb is None:
            raise SystemExit("--vram-gb is required when CUDA is used")
        total = torch.cuda.get_device_properties(0).total_memory / 2 ** 30
        torch.cuda.set_per_process_memory_fraction(min(1.0, args.vram_gb / total), 0)
        print(f"device cuda, {args.vram_gb:g} GB of {total:.1f} GB cap", flush=True)
    else:
        print("device cpu", flush=True)

    print(f"loading {args.ckpt}", flush=True)
    info = fixed_eval.load_ckpt(str(args.ckpt), dev)
    ppo.FORCE_DARK = info["force_dark_gift"]
    ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = info["old_boards"]
    ppo.REAL_LOBBY_FILTER = True
    print(f"  update {info['update']}, force_dark_gift {info['force_dark_gift']}, "
          f"old_boards {info['old_boards']}, comp inputs {info['n_comps']}", flush=True)

    started = time.time()
    bot_rows = []
    for lag in lags:
        lag_from = 1 if lag == 0 else args.lag2_from
        print(f"lag {lag} from turn {lag_from}", flush=True)
        bot_rows.extend(collect_bot(args, info, real, lag, lag_from, dev))
    real_rows = collect_real(folder)

    written = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    lags_text = ", ".join(
        f"lag {lag}" + ("" if lag == 0 else f" from turn {args.lag2_from}")
        for lag in lags)
    payload = {
        "checkpoint": str(args.ckpt),
        "update": info["update"],
        "written": written,
        "git": fixed_eval.git_rev(),
        "games": args.games,
        "seed_base": args.seed_base,
        "chunk": fixed_eval.CHUNK,
        "lineup": fixed_eval.LINEUP,
        "boards": str(folder),
        "n_board_files": n_board_files,
        "lags": lags,
        "lags_text": lags_text,
        "elapsed_seconds": round(time.time() - started, 1),
        "bot_rows": bot_rows,
        "real_rows": real_rows,
    }
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "rows.json").write_text(json.dumps(payload) + "\n")
    report = build_report(payload)
    (args.out_dir / "report.md").write_text(report)
    print(report)
    print(f"wrote {args.out_dir / 'rows.json'} and {args.out_dir / 'report.md'} "
          f"({len(bot_rows)} bot rows, {len(real_rows)} real rows, "
          f"{payload['elapsed_seconds']}s)", flush=True)


if __name__ == "__main__":
    main()
