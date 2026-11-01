"""Collect Era's shop phase actions from my real games into boards/actions/.

The logs `tools/collect_boards.py` archives (`~/bg-logs` and
`~/bg-logs-laptop`, read only) hold every Battlegrounds game I play. That
script reads the boards of every fight; this one reads the other half: what I
did in each shop phase, as named steps per turn, so a replay can walk my turns
and compare them with the engine.

    ~/projects/bg/.venv/bin/python tools/collect_actions.py [--rewrite]

One file per game, `boards/actions/<log folder>-g<N>.json`, numbered the way
`boards/real/` numbers its games. Like collect_boards.py it remembers in
`boards/actions/.state.json` which logs it has read and skips a log whose size
has not changed; `--rewrite` reads every log again.

What a turn holds (docs/real-boards.md, "The action file", has the shape):

- `start`: the state at the first moment the game lets me act in the turn:
  Gold, max Gold, Tavern tier, upgrade cost, hero powers, trinkets, the
  Tavern's offers in slot order (the spell slot included, and whether they are
  frozen), my hand and my board.
- `actions`: every step I took, in order, named after what I did, never our
  engine's action numbers: `buy`, `sell`, `play`, `activate` (a minion's or
  trinket's own Activate), `move` (dragging a minion on my board), `roll`,
  `freeze`, `level`, `hero_power`, `dark_discovery`, `trinket_pick`,
  `discover`, `end_turn`, and `other` for anything this script cannot name yet
  (counted, and printed by the run). Each step says what the client charged
  (`cost`, the card's or button's COST tag, when it has one) and what it
  actually took (`paid`: the Gold and the Health the log shows leaving), where
  its card went (`moved`: a Tavern slot to the hand, the hand to a board
  position, off the board), and what it changed: the cards that entered or
  left my hand and board with the card that made them, the cards whose numbers
  moved (a random buff's target), a Refresh's new offers, a Discover's options
  and the card picked.
- `end`: my hand, board and Gold when the turn ended, before the fight.

A step is measured against the state the step before it left, not against the
state at the moment I clicked: the game can change things between two of my
decisions (an end of turn effect, an effect while I am idle), and that way
nothing that happens in a turn is lost between two steps. Gold the game takes
with no step of mine is written on the step as `late_gold` and counted.

How the log says all this. Every step I take is logged twice: the game prints
my options (`GameState.DebugPrintOptions`: one per button or card I can click,
with the targets each one has) and then which one I picked
(`GameState.SendOption`, with the target and the board position). Discovers,
Choose Ones, Dark Discovery offers and trinket offers are a choice offered
(`DebugPrintEntityChoices`) and a pick (`SendChoices`). What the step changed
is in the tagged power log, which the tracker's parser
(`~/projects/bg/bg/powerlog.py`, read only) already turns into the live table
of entities. So nothing is read from our engine and nothing is guessed: this
script reads my decisions and diffs the entity table around each one.

Gold is `RESOURCES + TEMP_RESOURCES - RESOURCES_USED` on my player entity: what
the turn gave me, plus what a sell or a Gold effect put in the temporary pool,
minus what I have spent this turn. Health is my hero's `HEALTH - DAMAGE + ARMOR`
together, because a card priced in Health is paid out of Armor first (the log
says so with `META_DATA SPEND_HEALTH`, and the Armor is what drops).

The entity references an options dump prints carry a stale zone and position
(the dump is printed before the client applies the last change), so a step's
card, its position and its target are always looked up by entity id in the
live table at the moment I picked it.

Where the log cannot tell something the field is null and a counter of the
game's unread things goes up (`unread` in the file); the run prints them. Today
that is 24 cards whose CREATOR the log leaves empty, one turn I took no step
in, and one change the game made after my last step, across 47 games. No player
names are stored.

`tools/check_actions.py` reads these files back: it adds up the Gold and the
cards of every turn against what the turn started and ended with.
"""

import json
import re
import sys
from collections import Counter
from pathlib import Path

TRACKER = Path.home() / "projects/bg"
sys.path.insert(0, str(TRACKER))
from bg.battlegrounds import (Tracker, card_name, gifts_by_minion,  # noqa: E402
                              hero_enchants_by_minion, minion, trinkets)
from bg.powerlog import PowerLog  # noqa: E402

# The archives, and the file names collect_boards.py writes in boards/real/.
ARCHIVE = Path.home() / "bg-logs"
LAPTOP_ARCHIVE = Path.home() / "bg-logs-laptop"
OUT = Path(__file__).resolve().parent.parent / "boards/actions"
STATE = OUT / ".state.json"

# What a step's diff compares about a card: its numbers, its gifts, and its
# card id, which changes when three copies turn golden.
KEYS = ("card_id", "attack", "health", "golden", "keywords", "dark_gifts")
SHOP_TYPES = ("MINION", "BATTLEGROUND_SPELL")

# The buttons and slots my options point at.
DRAG_BUY = ("TB_BaconShop_DragBuy", "TB_BaconShop_DragBuy_Spell")
SELL = "TB_BaconShop_DragSell"
ROLL = "TB_BaconShop_8p_Reroll_Button"
FREEZE = "TB_BaconShopLockAll_Button"
LEVEL = "TB_BaconShopTechUp"
DARK = "BG36_Button_DarkGift"
TRINKET_SLOTS = {"BG30_Trinket_1st": "lesser", "BG30_Trinket_2nd": "greater"}
DARK_DISCOVERY = "BG36_MidGameEffect_010"  # what offers a Dark Discovery
TRIPLE = "TB_BaconShop_Triples_01"         # a triple's reward, played from hand
# Steps whose price the client puts in a COST tag of the card or button.
COSTED = ("buy", "roll", "level", "hero_power", "dark_discovery")
# The cards that make every Tavern minion and say nothing about where a card
# came from: the shop's own enchantment and the tag transfer between players.
# A card's `from` is left out for those only; the game's triple check
# (TB_BaconShop_3ofKindChecke) stays, since it says a triple made the card.
SYSTEM = ("TB_BaconShop_8P_PlayerE", "Bacon_TagTransferPlayerE", "PET_")
# Card types that are never a card of mine in hand during a shop phase.
NOT_IN_HAND = ("ENCHANTMENT", "PLAYER", "HERO", "GAME", "HERO_POWER")

# A card the log names, e.g. "[entityName=Wrath Weaver id=434 zone=PLAY
# zonePos=2 cardId=BGS_004 player=15]". A name can hold brackets of its own
# ("UNKNOWN ENTITY [cardType=INVALID]"), so the name part is greedy.
REF = re.compile(r"\[entityName=(.*) id=(\d+) zone=(\S+) zonePos=(\d+) cardId=(\S*) player=(\d+)\]$")
# The GameState calls the tracker's parser drops: it reads only the power log.
LINE = re.compile(r"^D \S+ GameState\.(\w+)\(\) - (.*)$")
OPTIONS_ID = re.compile(r"^id=(\d+)$")
OPTION = re.compile(r" {2}option (\d+) type=(\w+) mainEntity=(.*?) error=(\w+) errorParam=(.*)$")
OPTION_TARGET = re.compile(r" {4}target (\d+) entity=(.*?) error=(\w+) errorParam=(.*)$")
CHOICE_ID = re.compile(r"^id=(\d+) Player=.* TaskList=\d* ChoiceType=(\w+) CountMin=\d+ CountMax=\d+$")
CHOICE_SOURCE = re.compile(r"^ {2}Source=(.*)$")
CHOICE_ENTITY = re.compile(r"^ {2}Entities\[(\d+)\]=(.*)$")
CHOSEN_ID = re.compile(r"^id=(\d+) ChoiceType=(\w+)$")
CHOSEN_ENTITY = re.compile(r"^ {2}m_chosenEntities\[(\d+)\]=(.*)$")
SEND_OPTION = re.compile(
    r"^selectedOption=(-?\d+) selectedSubOption=(-?\d+) selectedTarget=(\d+) selectedPosition=(\d+)$")
BLOCK_START = re.compile(
    r"BLOCK_START BlockType=(\S+) Entity=(.+?) EffectCardId=\S* EffectIndex=(-?\d+) "
    r"Target=(.+?) SubOption=(-?\d+)(?: TriggerKeyword=\S*)?$")
# The block that shows a click on one of my board minions is an Activate or a
# drag to a new position.
BOARD_BLOCKS = ("PLAY", "MOVE_MINION")


def parse_ref(text):
    """The parts of an entity the log prints it as, or None."""
    m = REF.search(text)
    if not m:
        return None
    return {"id": int(m[2]), "zone": m[3], "pos": int(m[4]), "card_id": m[5],
            "player": int(m[6])}


def numbers(tags):
    """A card's {0} {1} {2} {3} (TAG_SCRIPT_DATA_NUM_1..4), or None when it has
    none. The same read collect_boards.py does."""
    sd = [tags.get(f"TAG_SCRIPT_DATA_NUM_{i}", 0) for i in range(1, 5)]
    return sd if any(sd) else None


class ActionLog(PowerLog):
    """The tracker's parser, plus the two things an action needs and it drops:
    the target of a block, and my options and choices (which are not power
    lines). `on_line(game, kind, body)` gets those, `kind` being the log's own
    call name."""

    block_target = None  # the Target of the BLOCK_START just read

    def __init__(self, on_line=None, **kw):
        super().__init__(**kw)
        self.on_line = on_line

    def _power(self, body):
        self.block_target = None
        if m := BLOCK_START.match(body):
            self.block_target = parse_ref(m[4])
        super()._power(body)

    def feed(self, line):
        m = LINE.match(line.rstrip("\r\n"))
        if m and m[1] not in ("DebugPrintPower", "DebugPrintGame"):
            if self.on_line is not None:
                self.on_line(self.game, m[1], m[2])
            return
        super().feed(line)


class ActionTracker(Tracker):
    """The tracker, plus the record of my shop turns.

    Games are found the way collect_boards.py finds them (a new game files the
    last one, so the numbering matches), and a turn is bounded by the game's
    own TURN: an odd TURN is a shop phase, and `(TURN + 1) // 2` is the round.
    """

    def __init__(self):
        super().__init__()
        self.log = ActionLog(on_tag=self._on_tag, on_new_game=self._on_new_game,
                             on_block_start=self._on_block_start,
                             on_block_end=self._on_block_end, on_line=self._on_line)
        self.games = []         # one record per game, None for a game with no turn
        self.unread = Counter()  # what this game's log could not tell
        self.reset()

    def reset(self):
        self.record = {"turns": [], "hero": None}
        self.unread = Counter()
        self.turn = None       # the shop turn being built
        self.pending = None    # the step being resolved
        self.options = {}      # the last options dump: index -> option
        self.choice = None     # the last choice offered
        self.ended = False     # the turn's end has been taken
        self.last = None       # the state the last step left
        self.last_gold = 0      # the Gold the last step left
        self.last_health = 0    # and my hero's Health

    # -- games ------------------------------------------------------------

    def _on_new_game(self, g):
        if self.game is not None and self.is_battlegrounds:
            self.file_game()
        self.reset()
        super()._on_new_game(g)

    def file_game(self, need_fight=False):
        """File the game just read, under the number collect_boards.py gives it:
        one per game the log ends, None for a game with no shop turn. The last
        game of a log takes no number unless it got as far as a fight, which is
        where collect_boards.py draws the same line."""
        self.close_turn(False)
        if need_fight and not self.fights:
            return
        rec = self.record if self.record.get("turns") else None
        if rec is not None:
            rec["unread"] = dict(self.unread)
            rec["hero"] = self.my_hero()
        self.games.append(rec)
        self.reset()

    def last_game(self):
        if self.is_battlegrounds:
            self.file_game(need_fight=True)

    def my_hero(self):
        hero = self.hero_entity()
        if hero is None or not hero.card_id:
            return None
        return {"card_id": hero.card_id, "name": card_name(hero.card_id)}

    def dummy(self):
        """The stand-in player: it holds Bob's Tavern in the shop phase."""
        if self.game is None:
            return None
        return next((p for p in self.game.players if p not in self.game.real_players), None)

    # -- the tracker's hooks ----------------------------------------------

    def _on_tag(self, g, e, tag, value):
        super()._on_tag(g, e, tag, value)
        if e is not g.game_entity:
            return
        if tag == "TURN":
            self.set_turn(value)
        elif tag == "STEP" and value == "MAIN_END" and self.turn is not None and not self.ended:
            # the turn is over: this is where my End Turn click lands, and
            # where the end of turn effects resolve
            self.close_step()
            self.ended = True
            self.open_step({"step": "end_turn"})

    def _on_block_start(self, g, kind, e):
        super()._on_block_start(g, kind, e)
        p = self.pending
        if p is None or p["step"]["step"] != "board" or e is None or e.id != p["entity"]:
            return
        if kind in BOARD_BLOCKS:
            p["step"]["step"] = "activate" if kind == "PLAY" else "move"
            target = getattr(self.log, "block_target", None)
            if target is not None and not p["step"].get("target"):
                p["step"]["target"] = self.where(target)

    def _on_line(self, g, kind, body):
        if g is None:
            return
        if kind == "DebugPrintOptions":
            self.options_line(g, body)
        elif kind == "SendOption":
            self.send_option(g, body)
        elif kind == "DebugPrintEntityChoices":
            self.choice_line(g, body)
        elif kind == "SendChoices":
            self.send_choices(g, body)

    # -- turns ------------------------------------------------------------

    def set_turn(self, value):
        if value % 2 == 1:
            self.close_turn(False)
            self.turn = {"turn": (value + 1) // 2, "start": None, "actions": [], "end": None}
            self.ended = False
        else:
            self.close_turn(True)

    def close_turn(self, before_fight):
        """Close the turn: the last step, then my hand, board and Gold as they
        were when the fight began (`before_fight`), or None when the game or
        the log ended before one."""
        if self.turn is None:
            return
        self.close_step()
        if self.turn["start"] is None:
            self.unread["a turn the log gives no decision in"] += 1
            self.turn = None
            return
        if not self.turn["actions"]:
            self.unread["a turn I took no step in"] += 1
            self.turn = None
            return
        end = self.state()
        # the game can change things after my last step (an end of turn effect
        # that runs once the turn is over, an effect while I am idle). Those
        # changes belong to the turn, so they are written into its last step
        # and counted, rather than lost.
        if self.last is not None:
            late = self.last_gold - self.gold()
            if late:
                # Gold the game took with no step of mine (an end of turn
                # effect, a trinket, a hero power that pays by itself)
                self.turn["actions"][-1]["late_gold"] = late
                self.unread["Gold the game took with no step of mine"] += 1
            if not self.same(self.last, end):
                self.diff(self.turn["actions"][-1], self.last, end, set())
                self.unread["a change after my last step"] += 1
        self.last = end
        self.last_gold = self.gold()
        self.last_health = self.health()
        end["gold"] = self.gold()
        end["health"] = self.hero_health()
        end["armor"] = self.armor()
        self.turn["end"] = end if before_fight else None
        self.record["turns"].append(self.turn)
        self.turn = None

    def same(self, a, b):
        """Whether two states hold the same cards, numbers and Tavern."""
        for part in ("hand", "board"):
            if [self.key(c) for c in a[part]] != [self.key(c) for c in b[part]]:
                return False
        if [self.key(c) for c in a["tavern"]] != [self.key(c) for c in b["tavern"]]:
            return False
        return a["hand_hidden"] == b["hand_hidden"]

    # -- my decisions -----------------------------------------------------

    def options_line(self, g, body):
        if OPTIONS_ID.fullmatch(body):
            self.decision()
            self.options = {}
            return
        if m := OPTION.search(body):
            self.options[int(m[1])] = {"index": int(m[1]), "type": m[2],
                                       "entity": parse_ref(m[3]), "targets": {}}
        elif m := OPTION_TARGET.search(body):
            if self.options:
                self.options[max(self.options)]["targets"][int(m[1])] = parse_ref(m[2])

    def decision(self):
        """The game is asking me what to do: close the last step and, in a new
        turn, take the state I was first offered."""
        self.close_step()
        if self.turn is not None and self.turn["start"] is None:
            self.begin()

    def begin(self):
        """The turn's start: the state the client first shows me, which is also
        what the first step of the turn is measured against."""
        self.turn["start"] = self.start_state()
        self.last = self.turn["start"]
        self.last_gold = self.turn["start"]["gold"]
        self.last_health = self.health()

    def send_option(self, g, body):
        m = SEND_OPTION.match(body)
        if not m:
            return
        index, sub, target = int(m[1]), int(m[2]), int(m[3])
        self.close_step()
        if self.turn is None:
            self.unread["a step outside a shop turn"] += 1
            return
        if self.turn["start"] is None:
            self.begin()
        opt = self.options.get(index)
        if opt is None:
            self.unread["a step the options dump does not list"] += 1
            self.open_step({"step": "other", "detail": "not in the options dump"})
            return
        ent = (opt["entity"] or {}).get("id")
        at = self.target_of(opt, target)
        step = self.classify(ent, at, opt)
        step["option"] = index
        if sub >= 0:
            step["sub_option"] = sub
        self.open_step(step, ent)

    def target_of(self, opt, target):
        if not target:
            return None
        return self.where(opt["targets"].get(target) or {"id": target})

    def where(self, r):
        """An entity named by where it sits: my board, the Tavern, my hand or my
        hero. Read from the live table by id, since an options dump's zones and
        positions can lag a step behind."""
        g = self.game
        me, dummy = self.me, self.dummy()
        e = g.entities.get(r.get("id")) if r else None
        if e is None:
            if not r:
                return None
            out = {"id": r["id"], "card_id": r.get("card_id") or None,
                   "name": card_name(r["card_id"]) if r.get("card_id") else None,
                   "kind": (r.get("zone") or "unknown").lower()}
            if r.get("pos"):
                out["position"] = r["pos"]
            return out
        out = {"id": e.id, "card_id": e.card_id or None,
               "name": card_name(e.card_id) if e.card_id else None}
        zone, ct = e["ZONE"], e["CARDTYPE"]
        if zone == "PLAY" and e["CONTROLLER"] == me:
            if ct == "HERO":
                out["kind"] = "hero"
            elif ct == "MINION":
                out["kind"] = "board"
                out["position"] = e["ZONE_POSITION"]
            else:
                out["kind"] = (ct or "play").lower()
        elif zone == "PLAY" and e["CONTROLLER"] == dummy:
            out["kind"] = "slot"
            out["slot"] = e["ZONE_POSITION"]
        elif zone == "HAND" and e["CONTROLLER"] == me:
            out["kind"] = "hand"
            out["index"] = e["ZONE_POSITION"]
        else:
            out["kind"] = (zone or "unknown").lower()
        return out

    def classify(self, ent, at, opt):
        """Name a step from the option I picked. The card is read from the live
        table: an option's own zone and position can lag one step behind."""
        g = self.game
        e = g.entities.get(ent) if ent else None
        cid = (e.card_id if e is not None else "") or ""
        ct = e["CARDTYPE"] if e is not None else ""
        me, dummy = self.me, self.dummy()
        if opt["type"] == "END_TURN":
            return {"step": "end_turn"}
        if cid in DRAG_BUY:
            step = {"step": "buy", "target": at}
            if cid.endswith("_Spell"):
                step["spell"] = True
            return step
        if cid == SELL:
            return {"step": "sell", "target": at}
        if cid == ROLL:
            return {"step": "roll"}
        if cid == FREEZE:
            return {"step": "freeze"}
        if cid.startswith(LEVEL):
            return {"step": "level"}
        if cid == DARK:
            return {"step": "dark_discovery"}
        if e is None:
            return {"step": "other", "detail": "an option with no card"}
        zone = e["ZONE"]
        if zone == "PLAY" and e["CONTROLLER"] == dummy and ct in SHOP_TYPES:
            # a click on the minion itself buys it, as dragging does
            step = {"step": "buy", "target": self.where({"id": e.id})}
            if ct == "BATTLEGROUND_SPELL":
                step["spell"] = True
            return step
        if ct == "HERO_POWER" and e["CONTROLLER"] == me:
            second = bool(e["ADDITIONAL_HERO_POWER_INDEX"])
            return {"step": "hero_power", "which": "second" if second else "main",
                    "target": at}
        if zone == "HAND" and e["CONTROLLER"] == me:
            return {"step": "play", "target": at}
        if zone == "PLAY" and e["CONTROLLER"] == me:
            # a minion, a trinket or a reward of mine: Activate or drag, which
            # the block that follows says
            return {"step": "board", "target": at}
        return {"step": "other", "detail": "an option the script does not name",
                "card_type": ct or None, "target": at}

    def choice_line(self, g, body):
        if m := CHOICE_ID.match(body):
            if m[2] != "GENERAL":
                self.choice = None  # the mulligan's hero picks and cards
                return
            self.decision()
            self.choice = {"id": int(m[1]), "source": None, "entities": {}, "pick": None}
            return
        if self.choice is None:
            return
        if m := CHOICE_SOURCE.match(body):
            self.choice["source"] = parse_ref(m[1])
        elif m := CHOICE_ENTITY.match(body):
            self.choice["entities"][int(m[1])] = parse_ref(m[2])

    def send_choices(self, g, body):
        if CHOSEN_ID.match(body):
            self.close_step()
            if self.choice is None or self.turn is None:
                return
            if self.turn["start"] is None:
                self.begin()
            self.open_step(self.choice_step())
            return
        if (m := CHOSEN_ENTITY.match(body)) and self.choice is not None:
            self.choice["pick"] = parse_ref(m[2])

    def choice_step(self):
        """The step a choice is written as. Which one it is comes from what the
        game says offered it: a trinket slot, the Dark Discovery, a triple's
        reward, or a card's own Discover or Choose One."""
        src = self.choice["source"] or {}
        cid = src.get("card_id") or ""
        e = self.game.entities.get(src["id"]) if src.get("id") else None
        step = {"step": "discover", "source": self.card_of(src),
                "options": [self.card_of(r) for _, r in sorted(self.choice["entities"].items())]}
        if cid in TRINKET_SLOTS:
            step["step"] = "trinket_pick"
            step["slot_kind"] = TRINKET_SLOTS[cid]
        elif cid == DARK_DISCOVERY:
            step["kind"] = "dark_gift"
        elif cid == TRIPLE:
            step["kind"] = "triple_reward"
        elif e is not None and e["CHOOSE_ONE"]:
            step["kind"] = "choose_one"
        else:
            step["kind"] = "discover"
        return step

    def card_of(self, r):
        """A card as a step names it: id, name, and what else the log shows of
        it (its Dark Gift, its stats, its price)."""
        if not r:
            return None
        g = self.game
        e = g.entities.get(r.get("id")) if r.get("id") else None
        out = {"id": r.get("id"), "card_id": r.get("card_id") or None,
               "name": card_name(r["card_id"]) if r.get("card_id") else None}
        if e is None:
            return out
        own = list(gifts_by_minion(g).get(e.id, ()))
        pointed = g.entities.get(e["DARK_GIFT_ENTITY"]) if e["DARK_GIFT_ENTITY"] else None
        if pointed is not None and pointed.card_id and pointed.card_id not in own:
            own.insert(0, pointed.card_id)
        if own:
            out["dark_gifts"] = [{"card_id": c, "name": card_name(c)} for c in own]
        if e["CARDTYPE"] == "MINION":
            out["attack"] = e["ATK"]
            out["health"] = e["HEALTH"] - e["DAMAGE"]
            out["golden"] = bool(e["PREMIUM"]) or e.card_id.endswith("_G")
        if e["COST"]:
            out["cost"] = e["COST"]
        return out

    # -- steps ------------------------------------------------------------

    def open_step(self, step, ent=None):
        """Open a step: the card it acts on, the Gold and Health it starts
        from, and the state it starts from."""
        # a step is measured against the state the last decision saw, so that
        # nothing that happens between two decisions is lost
        before = self.last if self.last is not None else self.state()
        if step["step"] in ("buy", "sell") and step.get("target"):
            ent = step["target"].get("id")
        step["paid"] = None
        self.pending = {"step": step, "entity": ent,
                        "gold": self.last_gold if self.last is not None else self.gold(),
                        "health": self.last_health if self.last is not None else self.health(),
                        "before": before}
        if ent is not None and step["step"] in ("buy", "play", "sell", "activate", "board"):
            step.update(self.place(ent, before))
            self.pending["entity"] = ent
        if step["step"] in COSTED and self.pending.get("entity"):
            e = self.game.entities.get(self.pending["entity"])
            # a minion's COST tag is not what the Tavern charges for it (that is
            # 3 Gold by the game's rule), so only a spell's price is read here
            if e is not None and e["COST"] and (step["step"] != "buy" or step.get("spell")):
                step["cost"] = e["COST"]
            if e is not None and step["step"] == "roll" and e["BACON_FREE_REFRESH_COUNT"]:
                # a reroll from a pool of free ones: the button keeps its
                # price but takes nothing
                step["free"] = e["BACON_FREE_REFRESH_COUNT"]

    def place(self, ent, state):
        """Where the card a step acts on sits, from the state the step starts
        from: a Tavern slot for a buy, a hand index or a board position."""
        for key, field in (("tavern", "slot"), ("hand", "index"), ("board", "position")):
            for c in state[key]:
                if c["id"] == ent:
                    out = {field: c[field], "card": self.card_brief(c)}
                    if key == "tavern" and "cost" in c:
                        out["card"]["cost"] = c["cost"]
                    return out
        return {}

    def moved_to(self, ent, state):
        """Where a card sits in a state: its hand index, its board position, its
        Tavern slot, or off when it is in none of the three (sold, consumed, or
        a spell that is done)."""
        for key, field in (("hand", "index"), ("board", "position"), ("tavern", "slot")):
            for c in state[key]:
                if c["id"] == ent:
                    return {"where": key, field: c[field]}
        return {"where": "off"}

    def card_brief(self, c):
        d = {"id": c["id"], "card_id": c["card_id"], "name": c["name"]}
        for k in ("attack", "health", "golden", "dark_gifts", "numbers"):
            if k in c:
                d[k] = c[k]
        return d

    def close_step(self):
        p, self.pending = self.pending, None
        if self.turn is None or p is None:
            # nothing to write this state into: the next step is measured from
            # the state the last step left, so nothing in between is lost
            return
        after = self.state()
        self.last = after
        self.last_gold = self.gold()
        self.last_health = self.health()
        step = p["step"]
        step["paid"] = {"gold": p["gold"] - self.gold(),
                        "health": p["health"] - self.health()}
        if step["step"] == "other" and step.get("target") is None:
            step.pop("target", None)
        if p.get("entity") is not None:
            self.move_of(step, p["entity"], p["before"], after)
        if step["step"] in ("discover", "trinket_pick"):
            picked = self.choice["pick"] if self.choice else None
            step["picked"] = self.card_of(picked)
            if picked is None:
                self.unread["a pick the log does not show"] += 1
            else:
                # where the card I picked went: my hand, my board, or a trinket
                # slot, which is neither
                now = self.moved_to(picked["id"], after)
                step["moved"] = {"id": picked["id"], "from": {"where": "offer"},
                                 "to": None if now["where"] == "off" else now}
            self.choice = None
        elif step["step"] == "freeze":
            step["on"] = self.frozen()
        elif step["step"] == "roll":
            step["tavern"] = after["tavern"]
        elif step["step"] == "level":
            step["tier_after"] = self.player()["PLAYER_TECH_LEVEL"]
        elif step["step"] == "move":
            moved = [c for c in after["board"] if c["id"] == p.get("entity")]
            if not moved:
                self.unread["a move the log does not finish"] += 1
            elif moved[0]["position"] == step.get("position"):
                self.turn["canceled"] = self.turn.get("canceled", 0) + 1
                return
            else:
                step["to"] = moved[0]["position"]
        self.diff(step, p["before"], after, self.acting(p))
        if step["step"] == "board" and self.changed(step):
            self.unread["a click on my own board that no block explains"] += 1
            step["step"] = "other"
            step["detail"] = "a click on my own board that no block explains"
        if step["step"] == "board" or self.canceled(step):
            self.turn["canceled"] = self.turn.get("canceled", 0) + 1
            return
        self.turn["actions"].append(step)

    def changed(self, step):
        """Whether a step took or changed anything at all."""
        if step["paid"] != {"gold": 0, "health": 0}:
            return True
        return any(k in step for k in ("entered", "left", "changed", "tavern",
                                       "hand_hidden", "picked", "moved"))

    def canceled(self, step):
        """A click that took nothing and changed nothing: picking a minion up
        and putting it back, inspecting one, or the like. It is not a step, so
        it is counted apart rather than replayed."""
        if step["step"] not in ("buy", "sell", "play", "activate", "discover",
                                "trinket_pick", "dark_discovery"):
            return False
        return not self.changed(step)

    def move_of(self, step, ent, before, after):
        """Where the card the step acted on went, when it went anywhere: a bought
        card from a Tavern slot to the hand, a played card from the hand to a
        board position, a sold card off the board."""
        was = self.moved_to(ent, before)
        now = self.moved_to(ent, after)
        if was == now:
            return
        step["moved"] = {"id": ent, "from": was,
                         "to": None if now["where"] == "off" else now}

    def acting(self, p):
        """The entities this step itself acted on, which the diff leaves out of
        what it reports: the card bought or sold, the card played, the card
        picked."""
        ids = set()
        if p.get("entity"):
            ids.add(p["entity"])
        if p["step"].get("picked"):
            ids.add(p["step"]["picked"]["id"])
        return ids

    def diff(self, step, before, after, acting):
        """What the step changed: the cards that entered or left my hand and
        board (with the card that made them, when the log says), the cards
        whose numbers moved, and the Tavern when a slot changed."""
        b_all = {c["id"]: c for c in before["board"] + before["hand"]}
        a_all = {c["id"]: c for c in after["board"] + after["hand"]}
        b_hand = {c["id"] for c in before["hand"]}
        a_hand = {c["id"] for c in after["hand"]}
        entered, left = [], []
        for cid, c in a_all.items():
            if cid in b_all:
                continue
            if cid in acting:
                continue
            d = self.brief(c, "hand" if cid in a_hand else "board")
            if "from" not in d:
                self.unread["a card came in with no card behind it"] += 1
            entered.append(d)
        for cid, c in b_all.items():
            if cid in a_all:
                continue
            if cid in acting:
                continue
            left.append(self.brief(c, "hand" if cid in b_hand else "board"))
        if entered:
            step["entered"] = entered
        if left:
            step["left"] = left
        changed = self.moved(before, after, "board", "position")
        changed += self.moved(before, after, "hand", "index")
        if changed:
            step["changed"] = changed
        if [self.key(c) for c in before["tavern"]] != [self.key(c) for c in after["tavern"]]:
            step["tavern"] = after["tavern"]
        if after["hand_hidden"] != before["hand_hidden"]:
            step["hand_hidden"] = after["hand_hidden"]

    def brief(self, c, where):
        d = {"id": c["id"], "where": where, "card_id": c["card_id"], "name": c["name"]}
        if where == "board":
            d["position"] = c["position"]
        else:
            d["index"] = c["index"]
        if "from" in c:
            d["from"] = c["from"]
        return d

    def moved(self, before, after, part, key):
        """The cards whose numbers or gifts changed: a random buff's target."""
        old = {c["id"]: c for c in before[part]}
        out = []
        for c in after[part]:
            b = old.get(c["id"])
            if b is None:
                continue
            was = {k: b.get(k) for k in KEYS}
            now = {k: c.get(k) for k in KEYS}
            if was != now:
                out.append({key: c[key], "card_id": c["card_id"], "name": c["name"],
                            "was": was, "now": now})
        return out

    def key(self, c):
        return (c["id"], c["card_id"], c.get("attack"), c.get("health"), c.get("golden"),
                c.get("keywords"), c.get("dark_gifts"), c.get("frozen"))

    # -- the states -------------------------------------------------------

    def start_state(self):
        """A turn's start: the state, plus the numbers the client shows above
        the board."""
        s = self.state()
        p = self.player()
        s.update({"gold": self.gold(), "gold_max": p["RESOURCES"] if p is not None else None,
                  "tier": p["PLAYER_TECH_LEVEL"] if p is not None else None,
                  "upgrade_cost": self.upgrade_cost(),
                  "health": self.hero_health(), "armor": self.armor(),
                  "hero_powers": self.hero_powers(),
                  "trinkets": self.trinket_list(),
                  "frozen": self.frozen()})
        return s

    def state(self):
        """My hand and board and the Tavern, each card as boards/real/ writes it
        plus where it sits."""
        g = self.game
        me, dummy = self.me, self.dummy()
        gifts, ench = gifts_by_minion(g), hero_enchants_by_minion(g)
        board, hand, tavern = [], [], []
        hidden = 0
        for e in g.entities.values():
            ct, zone = e["CARDTYPE"], e["ZONE"]
            if e["CONTROLLER"] == me and zone == "PLAY" and ct == "MINION":
                board.append(self.one(e, g, gifts, ench, "position"))
            elif e["CONTROLLER"] == dummy and zone == "PLAY" and ct in SHOP_TYPES:
                d = self.one(e, g, gifts, ench, "slot")
                d["kind"] = "spell" if ct == "BATTLEGROUND_SPELL" else "minion"
                if e["COST"]:
                    # only a Tavern spell carries its Gold price in the log: a
                    # minion costs 3 by the game's rule, not by a tag
                    d["cost"] = e["COST"]
                d["frozen"] = bool(e["FROZEN"])
                if e["BACON_COSTS_HEALTH_TO_BUY"]:
                    d["costs_health"] = True
                tavern.append(d)
            elif e["CONTROLLER"] == me and zone == "HAND" and ct not in NOT_IN_HAND:
                # a hand card the log does not name is kept, so that it can be
                # followed to where it goes: its card id is null and it is
                # counted in hand_hidden
                d = self.one(e, g, gifts, ench, "index")
                if not e.card_id:
                    hidden += 1
                if ct in SHOP_TYPES:
                    d["kind"] = "spell" if ct == "BATTLEGROUND_SPELL" else "minion"
                elif ct:
                    d["kind"] = ct.lower()
                hand.append(d)
        board.sort(key=lambda c: (c["position"], c["id"]))
        hand.sort(key=lambda c: (c["index"], c["id"]))
        tavern.sort(key=lambda c: (c["slot"], c["id"]))
        return {"board": board, "hand": hand, "tavern": tavern, "hand_hidden": hidden}

    def one(self, e, g, gifts, ench, key):
        d = {"id": e.id, key: e["ZONE_POSITION"]}
        if not e.card_id:
            d["card_id"] = None
            d["name"] = None
            return d
        m = minion(e, g, gifts, ench)
        d.update({"card_id": m.card_id, "name": m.name, "attack": m.attack,
                  "health": m.health, "golden": m.golden, "keywords": list(m.keywords)})
        if m.dark_gifts:
            d["dark_gifts"] = [{"card_id": c, "name": card_name(c)} for c in m.dark_gifts]
        sd = numbers(m.tags)
        if sd is not None:
            d["numbers"] = sd
        creator = g.entities.get(e["CREATOR"]) if e["CREATOR"] else None
        if creator is not None and creator.card_id and not creator.card_id.startswith(SYSTEM):
            d["from"] = {"card_id": creator.card_id, "name": card_name(creator.card_id)}
        return d

    # -- the client's numbers ---------------------------------------------

    def player(self):
        return self.game.player_entity(self.me) if self.game is not None and self.me is not None else None

    def hero_entity(self):
        p = self.player()
        return self.game.entities.get(p["HERO_ENTITY"]) if p is not None and p["HERO_ENTITY"] else None

    def gold(self):
        """The Gold the client shows: what the turn gave me, plus what a sell or
        a Gold effect put in the temporary pool (TEMP_RESOURCES), minus what I
        have spent this turn."""
        p = self.player()
        if p is None:
            return 0
        return p["RESOURCES"] + p["TEMP_RESOURCES"] - p["RESOURCES_USED"]

    def health(self):
        """Health and Armor together, which is what a card priced in Health
        takes: the client pays it out of Armor first (META_DATA SPEND_HEALTH in
        the log, 2026-10-24)."""
        hero = self.hero_entity()
        return hero["HEALTH"] + hero["ARMOR"] - hero["DAMAGE"] if hero is not None else 0

    def hero_health(self):
        """My hero's Health as the client shows it, Armor apart."""
        hero = self.hero_entity()
        return hero["HEALTH"] - hero["DAMAGE"] if hero is not None else 0

    def armor(self):
        hero = self.hero_entity()
        return hero["ARMOR"] if hero is not None else 0

    def upgrade_cost(self):
        """The price on the tier up button, or None at the top tier."""
        xs = [x for x in self.game.entities.values()
              if x["CONTROLLER"] == self.me and x["ZONE"] == "PLAY" and x.card_id.startswith(LEVEL)]
        return min(xs, key=lambda x: x.id)["COST"] if xs else None

    def hero_powers(self):
        xs = [x for x in self.game.entities.values()
              if x["CONTROLLER"] == self.me and x["ZONE"] == "PLAY" and x["CARDTYPE"] == "HERO_POWER"]
        out = []
        for x in sorted(xs, key=lambda x: x.id):
            d = {"card_id": x.card_id, "name": card_name(x.card_id),
                 "numbers": [x[f"TAG_SCRIPT_DATA_NUM_{i}"] for i in range(1, 5)],
                 "cost": x["COST"], "used_this_turn": x["HEROPOWER_ACTIVATIONS_THIS_TURN"]}
            if x["ADDITIONAL_HERO_POWER_INDEX"]:
                d["second"] = True
            out.append(d)
        return out

    def trinket_list(self):
        if self.me is None:
            return []
        return [{"card_id": t.card_id, "name": t.name, "greater": t.greater,
                 "numbers": list(t.numbers)} for t in trinkets(self.game, self.me)]

    def frozen(self):
        """The Tavern's lock: the Freeze button's own flag, else whether any
        offer is frozen."""
        btn = [x for x in self.game.entities.values()
               if x["CONTROLLER"] == self.me and x["ZONE"] == "PLAY" and x.card_id == FREEZE]
        if btn:
            return bool(btn[0]["TAG_SCRIPT_DATA_NUM_1"])
        dummy = self.dummy()
        return any(x["FROZEN"] for x in self.game.entities.values()
                   if x["CONTROLLER"] == dummy and x["ZONE"] == "PLAY" and x["CARDTYPE"] in SHOP_TYPES)


def read_log(path):
    """Yield (game number, game record) for every game in the log that has shop
    turns, numbered as collect_boards.py numbers them."""
    t = ActionTracker()
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            t.feed(line)
    t.last_game()
    for gi, rec in enumerate(t.games, 1):
        if rec:
            yield gi, rec


def main():
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
        if state.get(name) == size and "--rewrite" not in sys.argv:
            continue
        n, steps, unread = 0, Counter(), Counter()
        for gi, game in read_log(log):
            game = {"source": name, "game": gi, **game}
            (OUT / f"{name}-g{gi}.json").write_text(json.dumps(game, indent=1))
            n += 1
            for turn in game["turns"]:
                for a in turn["actions"]:
                    steps[a["step"]] += 1
            unread.update(game["unread"])
        print(f"{name}: {n} games, {dict(steps)}, unread {dict(unread)}", flush=True)
        state[name] = size
        STATE.write_text(json.dumps(state, indent=1))


if __name__ == "__main__":
    main()
