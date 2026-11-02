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

    ~/projects/bg/.venv/bin/python tools/collect_boards.py [--no-archive]

--no-archive skips copying the client's log folders into ~/bg-logs (it writes
outside this project), for a rerun from what is already archived.

A systemd user timer (bg-boards.timer) runs it every 15 minutes.
"""

import json
import shutil
import sys
from pathlib import Path

TRACKER = Path.home() / "projects/bg"
sys.path.insert(0, str(TRACKER))
from bg.battlegrounds import Tracker, minion  # noqa: E402

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


def numbers(tags):
    """A minion's current numbers, the {0} {1} {2} {3} of its text
    (TAG_SCRIPT_DATA_NUM_1..4), or None when it has none."""
    sd = [tags.get(f"TAG_SCRIPT_DATA_NUM_{i}", 0) for i in range(1, 5)]
    return sd if any(sd) else None


# Rapid Reanimation (Teron Gorefiend) marks the minion it destroys at Start of
# Combat with this enchantment, attached to it while both sit in PLAY. The
# board file writes a marked minion as "reanimate": true, which the engine's
# fight reads (docs/real-boards.md "The file").
REANIMATE_MARK = "BG25_HERO_103pe"

# The Secret cards a player can hold in the Secret zone (docs/real-boards.md
# "The file"): Venomstrike Trap, Autodefense Matrix, Redemption, Pack Tactics
# and the Ice Block a Safety Patch gives. The zone also holds the Deity and
# quests, which are not Secrets and are not written here.
SECRET_CARDS = ("TB_Bacon_Secrets_01", "TB_Bacon_Secrets_07", "TB_Bacon_Secrets_10",
                "TB_Bacon_Secrets_15", "TB_Bacon_Secrets_12")

# Tavish Stormpike's Lock and Load: the minion it stole from the Tavern is held
# in TAG_SCRIPT_DATA_ENT_1 of the power, and comes into play as a copy of it
# (COPIED_FROM_ENTITY_ID points at the held entity) when the fight has space.
LOCK_AND_LOAD = "BG22_HERO_000p_Alt"


def secret_cards_in_zone(g, pid):
    """The Secret card ids in player `pid`'s Secret zone, in the order they
    entered it (entity id order, which is the order they fire in)."""
    xs = [x for x in g.entities.values()
          if x["ZONE"] == "SECRET" and x["CONTROLLER"] == pid and x.card_id in SECRET_CARDS]
    return [x.card_id for x in sorted(xs, key=lambda x: x.id)]


def waiting_entity(g, pid):
    """The entity id of the minion Lock and Load still holds (0 for none). It
    is not created in the log when the opponent stole it, so its stats are read
    off the copy the power summons (see `CollectingTracker._on_tag`)."""
    hp = next((x for x in g.entities.values()
               if x.card_id == LOCK_AND_LOAD and x["ZONE"] == "PLAY" and x["CONTROLLER"] == pid), None)
    return hp["TAG_SCRIPT_DATA_ENT_1"] if hp is not None else 0


def reanimate_flags(g, pid):
    """Which minions on player `pid`'s board carry the mark, in board order (the
    order `board()` and the file use)."""
    marked = {x["ATTACHED"] for x in g.entities.values()
              if x.card_id == REANIMATE_MARK and x["ZONE"] == "PLAY" and x["ATTACHED"]}
    ms = [x for x in g.entities.values()
          if x["CARDTYPE"] == "MINION" and x["ZONE"] == "PLAY" and x["CONTROLLER"] == pid]
    return tuple(x.id in marked for x in sorted(ms, key=lambda x: x["ZONE_POSITION"]))


def minions(board, marks=()):
    out = []
    for i, m in enumerate(board):
        d = {"card_id": m.card_id, "name": m.name, "attack": m.attack,
             "health": m.health, "golden": m.golden, "keywords": list(m.keywords),
             "dark_gifts": list(m.dark_gifts)}
        sd = numbers(m.tags)
        if sd is not None:
            d["numbers"] = sd
        if i < len(marks) and marks[i]:
            d["reanimate"] = True
        out.append(d)
    return out


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
    trinkets and quest rewards (side 0 is me, 1 the opponent). The hero power
    also carries the minion Lock and Load still owes the board, as `waiting`
    (null when it owes none, which is also every power that is not Lock and
    Load)."""
    hp = fight.hero_powers[side]
    waiting = getattr(fight, "waiting", {}).get(side)
    return {
        "hero_power": {"card_id": hp.card_id, "name": hp.name,
                       "numbers": list(hp.numbers), "waiting": waiting}
        if hp else None,
        "trinkets": [{"card_id": t.card_id, "name": t.name, "greater": t.greater,
                      "numbers": list(t.numbers)} for t in fight.trinkets[side]],
        "rewards": [{"card_id": r.card_id, "name": r.name, "numbers": list(r.numbers)}
                    for r in fight.rewards[side]],
    }


# Player enchantments that carry a side-wide number, and what their script
# numbers are: the TAG_SCRIPT_DATA_NUM_1..4 slots holding attack and health
# (None for a number the enchant does not have). The log attaches them to the
# player of both seats (the opponent's to the stand-in player) and only while
# the number is nonzero, so a missing one means the base value.
PLAYER_ENCHANTS = {
    # Beetle Army: "Your Beetles have +{0}/+{1} this game" (Forest Rover,
    # Runed Progenitor, Turquoise Skitterer).
    "BG31_808pe": ("beetle_attack", "beetle_health"),
    # Undead Bonus Attack: "Your Undead have +{0} Attack this game, wherever
    # they are" (Nerubian Deathswarmer, Plaguerunner, Forsaken Weaver,
    # Butchering). There is no health one.
    "BG25_011pe": ("undead_attack", None),
    # Baller: how many Ballers were sold, the "improve your future Ballers"
    # count (Counter::Baller).
    "BG31_816pe": ("baller", None),
}


def enchant_numbers(g, pid, card_id):
    """The script numbers of one player enchantment `card_id` on player `pid`,
    or None when it is not there."""
    xs = [x for x in g.entities.values()
          if x.card_id == card_id and x["ZONE"] == "PLAY" and x["ATTACHED"] == pid]
    if not xs:
        return None
    x = max(xs, key=lambda x: x.id)
    return [x[f"TAG_SCRIPT_DATA_NUM_{i}"] for i in range(1, 5)]


def side_numbers(g, pid):
    """What the log shows of one side at the first attack besides its board:
    the player-level numbers a fight reads (`counters`, a key only when the
    log shows it) and its Deity (`deity`)."""
    player = g.player_entity(pid) if pid is not None else None
    counters = {}
    if player is not None:
        # The Tavern spell bonus: TAVERN_SPELL_ATTACK_INCREASE and
        # TAVERN_SPELL_HEALTH_INCREASE on the player entity.
        if player["TAVERN_SPELL_ATTACK_INCREASE"]:
            counters["spell_attack"] = player["TAVERN_SPELL_ATTACK_INCREASE"]
        if player["TAVERN_SPELL_HEALTH_INCREASE"]:
            counters["spell_health"] = player["TAVERN_SPELL_HEALTH_INCREASE"]
        for card_id, (atk_key, hp_key) in PLAYER_ENCHANTS.items():
            sd = enchant_numbers(g, player.id, card_id)
            if sd is None:
                continue
            if atk_key and sd[0]:
                counters[atk_key] = sd[0]
            if hp_key and sd[1]:
                counters[hp_key] = sd[1]
    return counters


# Upper Hand: "Start of Combat: Set a random enemy minion's Health to 1". The
# spell hides its pending trigger (the card goes back to SETASIDE), so the
# pending count is the Upper Hands each side *cast* since the turn began.
UPPER_HAND = "BG28_573"


class CollectingTracker(Tracker):
    """The tracker, plus the side-wide numbers at each snapshot it captures.

    `_capture` runs at the first attack (before any minion's own Start of
    Combat), which is the state the counters and the side's power are read
    from. The board itself is the one the tracker stages the fight with
    (`_start_fight`, before Start of Combat), so that is where Rapid
    Reanimation's mark is read: it lands on the minion the power chose, which
    is gone from the board by the first attack.
    """

    def __init__(self, *a, **kw):
        super().__init__(*a, **kw)
        # Upper Hands each side has cast this turn (see `UPPER_HAND`).
        self.upper_hands = [0, 0]

    def _on_block_start(self, g, kind, e):
        super()._on_block_start(g, kind, e)
        # An Upper Hand cast this turn is a pending Start of Combat trigger.
        # Its card goes back to SETASIDE, so the cast is all the log says.
        if kind == "PLAY" and e is not None and e.card_id == UPPER_HAND:
            side = 0 if e["CONTROLLER"] == self.me else 1
            self.upper_hands[side] += 1

    def _start_fight(self, g):
        # The board a file holds is the one before Start of Combat, which is
        # this instant, so this is where the state that only exists before the
        # fight is read: Rapid Reanimation's mark (at the first attack the
        # minion it marked is already destroyed and the enchantment sits on the
        # copy) and each side's Secret zone.
        stand = self._stand_in(g)
        self._waiting = {}
        if stand is None:
            return super()._start_fight(g)
        marks = (reanimate_flags(g, self.me), reanimate_flags(g, stand))
        secret_cards = (secret_cards_in_zone(g, self.me), secret_cards_in_zone(g, stand))
        # whose side a Lock and Load minion belongs to, by the entity the power
        # holds; the copy that comes in later names it in COPIED_FROM_ENTITY_ID
        for side, pid in ((0, self.me), (1, stand)):
            ent = waiting_entity(g, pid)
            if ent:
                self._waiting[ent] = side
        upper = (self.upper_hands[0], self.upper_hands[1])
        super()._start_fight(g)
        if self.fight is not None:
            self.fight.reanimates = marks
            self.fight.secrets = secret_cards
            self.fight.upper_hands = upper
        # A new turn's Upper Hands start from zero.
        self.upper_hands = [0, 0]

    def _on_tag(self, g, e, tag, value):
        super()._on_tag(g, e, tag, value)
        # Lock and Load's stolen minion is not in the log (the opponent's Tavern
        # is hidden), but the copy the power summons names it, with the stats it
        # had when it was taken. It is read when it appears, before it fights.
        if tag != "COPIED_FROM_ENTITY_ID" or not value:
            return
        side = self._waiting.get(value) if getattr(self, "_waiting", None) else None
        f = self._live
        if side is None or f is None or e["CARDTYPE"] != "MINION" or not e.card_id:
            return
        waiting = getattr(f, "waiting", None)
        if waiting is None:
            waiting = f.waiting = {}
        if side not in waiting:
            waiting[side] = minions([minion(e, g)])[0]

    def _capture(self, g):
        d = super()._capture(g)
        stand = self._stand_in(g)
        d["side_numbers"] = (side_numbers(g, self.me), side_numbers(g, stand))
        return d


def counters(fight, side):
    """A side's player-level numbers at the first attack, the keys the file
    holds. A number the log does not show is left out: the loader then uses
    the base value (0 spells cast, no spell bonus, Blood Gems +1/+1)."""
    # A fight nobody attacks in (two empty boards) never gets side_numbers.
    out = dict(getattr(fight, "side_numbers", ({}, {}))[side])
    if getattr(fight, "upper_hands", (0, 0))[side]:
        out["upper_hand"] = fight.upper_hands[side]
    # Blood Gems (BACON_BLOODGEMBUFFATKVALUE / _HEALTHVALUE on the player).
    ga, gh = fight.gems[side]
    if ga != 1 or gh != 1:
        out["gem_attack"] = ga
        out["gem_health"] = gh
    # The tracker's counts(): spells cast (NUM_SPELLS_PLAYED_THIS_GAME, mine
    # only: the stand-in player carries none) and Eternal Knight deaths (the
    # BG25_008pe player enchantment).
    if fight.counters[side].get("spells"):
        out["spells_cast"] = fight.counters[side]["spells"]
    if fight.counters[side].get("knights"):
        out["knight_deaths"] = fight.counters[side]["knights"]
    return out


def deity(fight, side):
    """The side's Deity at the first attack: the minion it awakens and the
    stats it has now (the tracker reads it off the BG_OldGod in SECRET)."""
    d = fight.deities[side]
    if d is None:
        return None
    return {"card_id": d.card_id, "name": d.name, "attack": d.attack, "health": d.health}


def side(fight, index, seat):
    """One side's board as the file holds it: the seat's hero, tier, health and
    armor, its hero power, trinkets and rewards, its minions, and what the log
    shows of the side itself at the first attack (counters, Deity, hand)."""
    out = dict(seat)
    out.update(extras(fight, index))
    marks = getattr(fight, "reanimates", ((), ()))[index]
    out["minions"] = minions(fight.mine if index == 0 else fight.theirs, marks)
    out["secrets"] = getattr(fight, "secrets", ([], []))[index]
    if fight.at_attack is not None:
        hand = fight.hand if index == 0 else fight.their_hand
        out["counters"] = counters(fight, index)
        out["deity"] = deity(fight, index)
        out["hand"] = minions(hand or ())
        out["hand_hidden"] = fight.hidden[index]
    return out


def read_log(path):
    """Yield (game number, game record) for every Battlegrounds game in the log."""
    t = CollectingTracker()
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
                           **side(fight, 0, mine)})
            boards.append({"who": "opp", "turn": fight.turn, "result": FLIP[fight.result],
                           **side(fight, 1, theirs)})
        if boards:
            yield gi, {"place": me.place if me else 0, "boards": boards}


def main():
    if "--no-archive" not in sys.argv:
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
