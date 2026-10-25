"""Collect the boards from my real games into boards/real/.

The game writes every Battlegrounds game to Logs/Hearthstone_<date>/Power.log
(the tracker doesn't need to be running) and deletes old log folders after a
few sessions. This script copies the log folders to ~/bg-logs/ so nothing is
lost, then reads each new or grown Power.log with the tracker's parser
(~/projects/bg, read only) and writes one JSON file per game:

    boards/real/<log folder>-g<N>.json

Every fight gives two boards, each marked with `who`:
- "me": my board, one per turn, so a game of mine can be followed turn by turn.
- "opp": the opponent I fought that turn, one random player per turn.

Player names are not stored. Run it with the tracker's venv:

    ~/projects/bg/.venv/bin/python tools/collect_boards.py

A systemd user timer (bg-boards.timer) runs it every 15 minutes.
"""

import json
import shutil
import sys
from pathlib import Path

TRACKER = Path.home() / "projects/bg"
sys.path.insert(0, str(TRACKER))
from bg.battlegrounds import Tracker  # noqa: E402

LOGS = Path.home() / "hearthstone-linux/hearthstone/Logs"
ARCHIVE = Path.home() / "bg-logs"
# The laptop keeps its own logs and rsyncs them here (~/.local/bin/bg-logs-sync
# on era-laptop); its boards get a "laptop-" prefix.
LAPTOP_ARCHIVE = Path.home() / "bg-logs-laptop"
OUT = Path(__file__).resolve().parent.parent / "boards/real"
STATE = OUT / ".state.json"


def archive():
    """Copy every log folder to the archive; a folder that grew is copied again."""
    ARCHIVE.mkdir(exist_ok=True)
    for d in sorted(LOGS.glob("Hearthstone_*")):
        src = d / "Power.log"
        if not src.exists():
            continue
        dst = ARCHIVE / d.name / "Power.log"
        if dst.exists() and dst.stat().st_size == src.stat().st_size:
            continue
        dst.parent.mkdir(exist_ok=True)
        shutil.copy2(src, dst)


def minions(board):
    return [
        {"card_id": m.card_id, "name": m.name, "attack": m.attack, "health": m.health,
         "golden": m.golden, "keywords": list(m.keywords),
         "dark_gifts": list(m.dark_gifts)}
        for m in board
    ]


def seat_info(seat):
    if seat is None:
        return {"hero": "", "hero_card": "", "hero_base": "", "tier": 0, "health": 0,
                "armor": 0}
    # hero_card can be a skin (BG23_HERO_303_SKIN_D); hero_base is the hero
    return {"hero": seat.hero, "hero_card": seat.hero_card,
            "hero_base": seat.hero_card.split("_SKIN_")[0], "tier": seat.tier,
            "health": seat.health, "armor": seat.armor}


FLIP = {"win": "lose", "lose": "win", "tie": "tie", "": ""}


def extras(fight, side):
    """What a side had at the first attack besides its board: hero power,
    trinkets and quest rewards (side 0 is me, 1 the opponent)."""
    hp = fight.hero_powers[side]
    return {
        "hero_power": {"card_id": hp.card_id, "name": hp.name, "numbers": list(hp.numbers)}
        if hp else None,
        "trinkets": [{"card_id": t.card_id, "name": t.name, "greater": t.greater,
                      "numbers": list(t.numbers)} for t in fight.trinkets[side]],
        "rewards": [{"card_id": r.card_id, "name": r.name, "numbers": list(r.numbers)}
                    for r in fight.rewards[side]],
    }


def read_log(path):
    """Yield (game number, game record) for every Battlegrounds game in the log."""
    t = Tracker()
    games = []  # per game: list of (fight, my seat, their seat) snapshots
    current = []
    seen_past = 0
    n_fights = 0
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            t.feed(line)
            if len(t.past) > seen_past:
                # a game ended: its fights are done
                seen_past = len(t.past)
                games.append((current, t.past[-1][0]))
                current = []
                n_fights = 0
            if len(t.fights) > n_fights:
                n_fights = len(t.fights)
                fight = t.fights[-1]
                # seats as they are at the first attack: tier, health, armor
                current.append((fight, seat_info(t.seats.get(t.me)),
                                seat_info(t.seats.get(fight.opponent))))
    if current and t.is_battlegrounds:
        games.append((current, t.seats))
    for gi, (fights, seats) in enumerate(games, 1):
        me = seats.get(t.me)
        boards = []
        for fight, mine, theirs in fights:
            if not fight.result:
                continue
            boards.append({"who": "me", "turn": fight.turn, "result": fight.result,
                           **mine, **extras(fight, 0), "minions": minions(fight.mine)})
            boards.append({"who": "opp", "turn": fight.turn, "result": FLIP[fight.result],
                           **theirs, **extras(fight, 1), "minions": minions(fight.theirs)})
        if boards:
            yield gi, {"place": me.place if me else 0, "boards": boards}


def main():
    archive()
    OUT.mkdir(parents=True, exist_ok=True)
    try:
        state = json.loads(STATE.read_text())
    except (OSError, ValueError):
        state = {}
    logs = [("", p) for p in sorted(ARCHIVE.glob("Hearthstone_*/Power.log"))]
    logs += [("laptop-", p) for p in sorted(LAPTOP_ARCHIVE.glob("Hearthstone_*/Power.log"))]
    for prefix, log in logs:
        name = prefix + log.parent.name
        size = log.stat().st_size
        if state.get(name) == size:
            continue
        for gi, game in read_log(log):
            game = {"source": name, "game": gi, **game}
            (OUT / f"{name}-g{gi}.json").write_text(json.dumps(game, indent=1))
        state[name] = size
        STATE.write_text(json.dumps(state, indent=1))


if __name__ == "__main__":
    main()
