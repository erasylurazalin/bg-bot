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
  frozen), my hand and my board, the player-level numbers a step reads
  (`counters`, the board files' keys plus Gold spent and the Tavern's "this
  game" buffs), my Secret zone (`secrets`) and the marks a fight or a shop step
  reads (a minion Rapid Reanimation chose carries `reanimate`; a power holding
  a minion carries it as `waiting`).
- `actions`: every step I took, in order, named after what I did, never our
  engine's action numbers: `buy`, `sell`, `play`, `activate` (a minion's or
  trinket's own Activate), `move` (dragging a minion on my board), `roll`,
  `freeze`, `level`, `hero_power`, `dark_discovery`, `trinket_pick`,
  `discover`, `end_turn`, and `other` for anything this script cannot name yet
  (counted, and printed by the run). Each step says what the client charged
  (`cost`, the card's or button's COST tag, when it has one) and what it
  actually took (`paid`: the Gold and the Health the log shows leaving), where
  its card went (`moved`: a Tavern slot to the hand, the hand to a board
  position, off the board), what it changed (the cards that entered or left my
  hand and board with the card that made them, the cards whose numbers moved,
  a Refresh's new offers, a Discover's options and the card picked), and what
  else it did: the Health it took and what gave it back (`health_cost`,
  `rewound`), the Gold it gained (`gold_gain`), the free Refreshes it used or
  that came in (`free_from`, `free_refreshes`).
- `after` on every step: the whole state the step left, in `start`'s shape.
  That is what the shop replay loads before the next step, and what it compares
  a step's result with (`bgbench shopreplay`, docs/real-boards.md "The shop
  replay"), so the log's randomness never has to be forced into the engine.
- `end`: my hand, board and Gold when the turn ended, before the fight. It is
  the same object as the last step's `after`.
- `tribes` on the game: the tribes the log showed me, taken from every minion
  that sat in my Tavern, my hand or my board. The lobby itself is not in the
  log, so this is a lower bound on it.

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

Three things a step can do besides spend Gold are read from the tags inside it,
and named by the card of the block they happen in: my hero's Damage or Armor
coming back (a Soul Rewinder or an Ashen Corruptor), my Gold going up (the
Minion it came from, a spell, a trinket), and the free Refresh count moving
(`BACON_FREE_REFRESH_COUNT` on `Bacon_Free_Refresh_Player_Ench`), which keeps
the cards that filled the pool so a free reroll can name one. A card that
enters my hand with no CREATOR is a card stolen out of the Tavern: it keeps its
entity id, so the step it entered in carries the Tavern slot it came from.

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
from bg.battlegrounds import (Tracker, card_name, cardxml, gifts_by_minion,  # noqa: E402
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
# The player enchantment that holds my free Refreshes (BACON_FREE_REFRESH_COUNT
# on it and on the Refresh button; the button's own tag is the same number, so
# only the enchantment is followed to avoid counting a grant twice).
FREE_REFRESH = "Bacon_Free_Refresh_Player_Ench"
# The card types a block's entity can have and still be the card that did
# something: an effect's own card, not a player or the game.
SOURCES = ("MINION", "SPELL", "BATTLEGROUND_SPELL", "BATTLEGROUND_TRINKET",
           "HERO_POWER", "ENCHANTMENT", "WEAPON")
# The tags above: a free Refresh changing hands, my hero taking or getting back
# Health, my Gold moving.
WATCHED = ("BACON_FREE_REFRESH_COUNT", "HEALTH", "DAMAGE", "ARMOR", "RESOURCES",
           "TEMP_RESOURCES", "RESOURCES_USED")

# Rapid Reanimation (Teron Gorefiend) marks the minion it destroys at Start of
# Combat with this enchantment, attached to it while both sit in PLAY. A state
# writes a marked minion as "reanimate": true, the key the board files use
# (docs/real-boards.md "The file").
REANIMATE_MARK = "BG25_HERO_103pe"

# The Secret cards a player can hold in the Secret zone, in the order they were
# got (the order they fire in): Venomstrike Trap, Autodefense Matrix,
# Redemption, Pack Tactics and the Ice Block a Safety Patch gives. The zone also
# holds the Deity and quests, which are not Secrets and are not written.
SECRET_CARDS = ("TB_Bacon_Secrets_01", "TB_Bacon_Secrets_07", "TB_Bacon_Secrets_10",
                "TB_Bacon_Secrets_15", "TB_Bacon_Secrets_12")

# Tavish Stormpike's Lock and Load: the minion it stole from the Tavern is held
# in TAG_SCRIPT_DATA_ENT_1 of the power and comes into play as a copy of it at
# the next Start of Combat. A hero power is written with it as `waiting` (null
# when the power owes nothing), the key the board files use.
LOCK_AND_LOAD = "BG22_HERO_000p_Alt"

# Player enchantments that carry a side-wide number a shop step reads, with the
# script numbers holding attack and health (None for a number the enchantment
# does not have). The first three are the board files' counters, read the same
# way collect_boards.py reads them. The last two are Tavern buffs ("minions in
# Bob's Tavern have +X/+Y this game"): the game keeps one enchantment per buff
# and adds into its numbers, so its current numbers are the whole buff.
PLAYER_ENCHANTS = {
    # Beetle Army: "Your Beetles have +{0}/+{1} this game".
    "BG31_808pe": ("beetle_attack", "beetle_health"),
    # Undead Bonus Attack: "Your Undead have +{0} Attack this game".
    "BG25_011pe": ("undead_attack", None),
    # Baller: Ballers sold, the "improve your future Ballers" count.
    "BG31_816pe": ("baller", None),
    # Staff of Enrichment and its kin: the whole Tavern has +{0}/+{1}.
    "BG_ShopBuff": ("tavern_attack", "tavern_health"),
    # Obsidian Ravager and its kin: Elementals in the Tavern have +{0}/+{1}.
    "BG_ShopBuff_Elemental": ("tavern_elemental_attack", "tavern_elemental_health"),
    # Waveling, En-Djinn Blazer, Easterly Winds: what one Refresh gives one
    # random Tavern minion ("after the Tavern is Refreshed this game").
    "BG34_854pe": ("refresh_attack", "refresh_health"),
    # Blood Gem Barrage casts: every Refresh gives this much, as Blood Gems, to
    # every minion in the Tavern.
    "BG34_689e2": ("barrage_attack", "barrage_health"),
}

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


# The ten tribes, named the way the engine spells them (cards.toml), from the
# card data's Race enum names.
TRIBE_NAMES = {"beast": "beast", "mechanical": "mech", "dragon": "dragon",
               "murloc": "murloc", "demon": "demon", "pirate": "pirate",
               "elemental": "elemental", "undead": "undead", "quilboar": "quilboar",
               "aberration": "aberration"}
_races = {}


def card_tribes(card_id):
    """The tribes of a card, as the engine names them. The lobby a game was
    played in is not in the log, so a game's tribes are the ones the log showed
    me: every tribe of every minion that sat in my Tavern, my hand or my board.
    That is a lower bound (a lobby often has a tribe the Tavern never offered),
    and a pool built from it can only draw fewer cards than the real one."""
    if not card_id:
        return ()
    if card_id not in _races:
        cards, _ = cardxml.load()
        c = cards.get(card_id)
        _races[card_id] = tuple(TRIBE_NAMES[r.name.lower()] for r in (c.races if c else ())
                                if r.name.lower() in TRIBE_NAMES)
    return _races[card_id]


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
        # Upper Hands cast this turn: "Start of Combat: set a random enemy
        # minion's Health to 1" hides its pending trigger, so the casts are the
        # count (the board files write it the same way).
        self.upper = 0
        self.unread = Counter()
        self.turn = None       # the shop turn being built
        self.pending = None    # the step being resolved
        self.options = {}      # the last options dump: index -> option
        self.choice = None     # the last choice offered
        self.ended = False     # the turn's end has been taken
        self.last = None       # the state the last step left
        self.last_gold = 0      # the Gold the last step left
        self.last_health = 0    # and my hero's Health
        self.free = 0           # free Refreshes I have
        self.pool = []          # who put them there: [{"count": n, "card": {...}}]
        self.stats = None       # what the step being resolved did besides spend
        self.damage = None      # my hero's DAMAGE at the last look
        self.armor_now = None   # and its Armor
        self.gold_live = None   # and my Gold
        self.waited = {}        # Lock and Load minions read off the summoned copy
        self.tribes = set()     # the tribes the log showed me this game

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
            # the lobby's tribes are not in the log; these are the ones it
            # showed me (see `card_tribes`)
            rec["tribes"] = sorted(self.tribes)
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
        self.watch(e, tag, value)
        if tag == "COPIED_FROM_ENTITY_ID" and value:
            self.waited_copy(e, value)
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

    def waited_copy(self, e, held):
        """The copy a power summons names the minion it held, in
        COPIED_FROM_ENTITY_ID, with the stats it had when it was taken. Only
        Lock and Load's held minion is not in the log itself (the Tavern copy
        it stole is), so this is the fallback `waiting_minion` reads."""
        if e["CARDTYPE"] != "MINION" or not e.card_id or held in self.waited:
            return
        m = minion(e, self.game)
        d = {"card_id": m.card_id, "name": m.name, "attack": m.attack,
             "health": m.health, "golden": m.golden, "keywords": list(m.keywords)}
        sd = numbers(m.tags)
        if sd is not None:
            d["numbers"] = sd
        self.waited[held] = d

    def watch(self, e, tag, value):
        """Follow, tag by tag, the things a step can do besides take Gold and
        spend it: free Refreshes coming in, Health taken and given back, Gold
        gained. The log says which card did each one by the block it is in."""
        if tag not in WATCHED:
            return
        if tag == "BACON_FREE_REFRESH_COUNT" and e.card_id.startswith(FREE_REFRESH):
            self.free_refreshes(value)
            return
        hero, me = self.hero_entity(), self.player()
        if hero is not None and e.id == hero.id and tag in ("HEALTH", "DAMAGE", "ARMOR"):
            if tag == "DAMAGE":
                # damage undone shows as my hero's DAMAGE going back down
                if self.damage is not None and value < self.damage and self.stats is not None:
                    self.add_restore(self.damage - value)
                self.damage = value
            elif tag == "ARMOR":
                # and, when the Armor took it (a Health price is paid out of
                # Armor first), as the Armor coming back
                if self.armor_now is not None and value > self.armor_now and self.stats is not None:
                    self.add_restore(value - self.armor_now)
                self.armor_now = value
            if self.stats is not None:
                # the deepest the Health ever went in this step: what it cost
                # before anything gave it back
                self.stats["floor"] = min(self.stats["floor"], self.health())
        elif me is not None and e.id == me.id and tag in ("RESOURCES", "TEMP_RESOURCES",
                                                          "RESOURCES_USED"):
            now = self.gold()
            if self.gold_live is not None and now > self.gold_live and self.stats is not None:
                self.stats["gold_gain"] += now - self.gold_live
                self.stats["gold_by"] = self.source() or self.stats["gold_by"]
            self.gold_live = now

    def add_restore(self, health):
        """My hero got Health or Armor back inside a step: remember how much and
        the card whose block did it (Soul Rewinder, Ashen Corruptor)."""
        self.stats["restored"] += health
        self.stats["restore_by"] = self.source() or self.stats["restore_by"]

    def free_refreshes(self, count):
        """The free Refresh count changed: remember who put the new ones there,
        and drop the ones I have spent (the log keeps only the total)."""
        gain = count - self.free
        self.free = count
        if gain > 0:
            self.pool.append({"count": gain, "card": self.source()})
        else:
            # the log only keeps the total, so the ones I spend come off the
            # oldest grant first
            spend = -gain
            while spend > 0 and self.pool:
                take = min(spend, self.pool[0]["count"])
                self.pool[0]["count"] -= take
                spend -= take
                if not self.pool[0]["count"]:
                    self.pool.pop(0)
        if not count:
            self.pool = []

    def source(self):
        """The card behind what is happening now: the innermost open block whose
        entity is an effect's own card."""
        for _, e in reversed(self._blocks):
            if e is None or not e.card_id or e.card_id.startswith(SYSTEM):
                continue
            if e["CARDTYPE"] in SOURCES:
                return {"card_id": e.card_id, "name": card_name(e.card_id)}
        return None

    def _on_block_start(self, g, kind, e):
        super()._on_block_start(g, kind, e)
        if kind == "PLAY" and e is not None and e.card_id == "BG28_573":
            self.upper += 1
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
            self.upper = 0
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
        end = self.full_state()
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
        # the turn's end is the state the last step left, with the changes the
        # game made after it: the next turn starts from the same shape
        self.turn["actions"][-1]["after"] = end
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
        self.turn["start"] = self.full_state()
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
            m = minion(e, g)
            out["attack"] = m.attack
            out["health"] = m.health
            out["golden"] = m.golden
            # An offer's card is written as it stands, keywords and numbers
            # included: a Discover's pick lands with them (a stat-set offer,
            # Innkeeper's Hearth, is not the card's printed stats).
            out["keywords"] = list(m.keywords)
            sd = numbers(m.tags)
            if sd is not None:
                out["numbers"] = sd
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
        health = self.last_health if self.last is not None else self.health()
        self.pending = {"step": step, "entity": ent,
                        "gold": self.last_gold if self.last is not None else self.gold(),
                        "health": health, "before": before,
                        "free": self.free, "pool": self.pool_cards()}
        hero = self.hero_entity()
        self.damage = hero["DAMAGE"] if hero is not None else None
        self.armor_now = hero["ARMOR"] if hero is not None else None
        self.gold_live = self.gold()
        self.stats = {"floor": health, "restored": 0, "restore_by": None,
                      "gold_gain": 0, "gold_by": None}
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
        after = self.full_state()
        self.last = after
        self.last_gold = self.gold()
        self.last_health = self.health()
        step = p["step"]
        # the state the step left, which is what the replay loads before the
        # next one and compares this one's result with
        step["after"] = after
        step["paid"] = {"gold": p["gold"] - self.gold(),
                        "health": p["health"] - self.health()}
        self.finish_watch(step, p)
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
        it is counted apart rather than replayed. A card that is still in the
        Tavern when the click is over is one of those: only a card that reached
        my hand was bought."""
        if step["step"] == "buy":
            to = (step.get("moved") or {}).get("to") or {}
            if to.get("where") == "hand" or "entered" in step:
                return False   # it reached my hand, or a triple took it
            return step["paid"] == {"gold": 0, "health": 0} and not step.get("gold_gain")
        if step["step"] not in ("sell", "play", "activate", "discover",
                                "trinket_pick", "dark_discovery"):
            return False
        return not self.changed(step)

    def finish_watch(self, step, p):
        """Write what the step did besides take Gold and spend it: the Health it
        cost and what gave it back, the Gold it gained, the free Refreshes it
        used, each with the card the log names."""
        st = self.stats or {}
        net = step["paid"]["health"]
        peak = round(p["health"] - st.get("floor", p["health"]))
        given_back = peak - max(net, 0)
        if peak > 0 and given_back > 0:
            # Health went down and came back: what it cost, and what gave it
            # back (Soul Rewinder, Ashen Corruptor)
            step["health_cost"] = peak
            step["rewound"] = {"health": given_back, "by": st.get("restore_by")}
            if st.get("restore_by") is None:
                self.unread["Health given back by no card I can name"] += 1
        if st.get("gold_gain") and step["step"] != "sell":
            step["gold_gain"] = {"gold": st["gold_gain"], "by": st.get("gold_by")}
        if self.free != p.get("free"):
            step["free_refreshes"] = {"before": p.get("free"), "after": self.free}
            if step["step"] == "roll" and not step["paid"]["gold"] and p.get("pool"):
                step["free_from"] = p["pool"]
        self.stats = None

    def pool_cards(self):
        """The cards that put the free Refreshes I hold in the pool, in order."""
        cards = []
        for g in self.pool:
            if g["card"] and g["card"] not in cards:
                cards.append(g["card"])
        return cards

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
                # a card stolen out of the Tavern keeps its entity, so its
                # CREATOR is empty and its Tavern slot says where it came from
                slot = next((t["slot"] for t in before["tavern"] if t["id"] == cid), None)
                if slot is not None:
                    d["stolen"] = {"where": "tavern", "slot": slot}
                else:
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

    def full_state(self):
        """A turnover point of the shop phase as the replay loads it: my hand,
        my board and the Tavern, the numbers the client shows above the board,
        the hero powers and trinkets, and the player-level numbers and the marks
        a step reads. One shape for a turn's `start`, every step's `after` and
        the turn's `end`, so a replay can load any of them the same way."""
        s = self.state()
        p = self.player()
        s.update({"gold": self.gold(), "gold_max": p["RESOURCES"] if p is not None else None,
                  "tier": p["PLAYER_TECH_LEVEL"] if p is not None else None,
                  "upgrade_cost": self.upgrade_cost(),
                  "health": self.hero_health(), "armor": self.armor(),
                  "hero_powers": self.hero_powers(),
                  "trinkets": self.trinket_list(),
                  "free_refreshes": self.free,
                  "frozen": self.frozen(),
                  "counters": self.counters(),
                  "secrets": self.secrets(),
                  "won_last_combat": self.won_last_combat()})
        return s

    def state(self):
        """My hand and board and the Tavern, each card as boards/real/ writes it
        plus where it sits."""
        g = self.game
        me, dummy = self.me, self.dummy()
        gifts, ench = gifts_by_minion(g), hero_enchants_by_minion(g)
        marks = self.reanimate_marks(g)
        board, hand, tavern = [], [], []
        hidden = 0
        for e in g.entities.values():
            ct, zone = e["CARDTYPE"], e["ZONE"]
            if e["CONTROLLER"] == me and zone == "PLAY" and ct == "MINION":
                d = self.one(e, g, gifts, ench, "position")
                if e.id in marks:
                    # marked by Rapid Reanimation, as the board files write it
                    d["reanimate"] = True
                self.tribes.update(card_tribes(e.card_id))
                board.append(d)
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
                self.tribes.update(card_tribes(e.card_id))
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
                self.tribes.update(card_tribes(e.card_id))
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
            # Lock and Load holds the Tavern minion it stole in the power's own
            # entity tag and summons a copy of it at the next Start of Combat.
            # Every other power owes nothing, and says so with a null (the
            # board files write the same key).
            d["waiting"] = self.waiting_minion(x)
            out.append(d)
        return out

    def waiting_minion(self, power):
        """The minion a power still owes the board, as a board minion, or None.
        Read off the held entity itself; when the log never created it, the
        copy the power summons names it (`COPIED_FROM_ENTITY_ID`, read in
        `_on_tag`), as collect_boards.py does at a fight."""
        ent = power["TAG_SCRIPT_DATA_ENT_1"]
        if not ent:
            return None
        held = self.game.entities.get(ent)
        if held is None or held["CARDTYPE"] != "MINION" or not held.card_id:
            return self.waited.get(ent)
        m = minion(held, self.game)
        d = {"card_id": m.card_id, "name": m.name, "attack": m.attack,
             "health": m.health, "golden": m.golden, "keywords": list(m.keywords)}
        sd = numbers(m.tags)
        if sd is not None:
            d["numbers"] = sd
        return d

    def trinket_list(self):
        if self.me is None:
            return []
        return [{"card_id": t.card_id, "name": t.name, "greater": t.greater,
                 "numbers": list(t.numbers)} for t in trinkets(self.game, self.me)]

    def secrets(self):
        """My Secret zone: the Secret card ids in it, in the order I got them
        (which is the order they fire in). Always written, empty when there is
        nothing."""
        xs = [x for x in self.game.entities.values()
              if x["ZONE"] == "SECRET" and x["CONTROLLER"] == self.me
              and x.card_id in SECRET_CARDS]
        return [x.card_id for x in sorted(xs, key=lambda x: x.id)]

    def reanimate_marks(self, g):
        """The entity ids of my minions Rapid Reanimation marked, the minion the
        power destroys at the next Start of Combat."""
        attached = {x["ATTACHED"] for x in g.entities.values()
                    if x.card_id == REANIMATE_MARK and x["ZONE"] == "PLAY" and x["ATTACHED"]}
        return {e.id for e in g.entities.values()
                if e["CARDTYPE"] == "MINION" and e["ZONE"] == "PLAY"
                and e["CONTROLLER"] == self.me and e.id in attached}

    def won_last_combat(self):
        """Whether my last fight was a win (the player tag of that name), which
        the cards that say "if you lost your last combat" read. None when the
        tag is not there (a game's first turn): the replay leaves it unknown."""
        p = self.player()
        if p is None or "BACON_WON_LAST_COMBAT" not in p.tags:
            return None
        return bool(p["BACON_WON_LAST_COMBAT"])

    def counters(self):
        """The player-level numbers a shop step reads, by the board files' keys
        (docs/real-boards.md "The file"), plus the ones only a shop step reads:
        Gold spent this game and the Tavern's "this game" buffs. A key is
        written only when the log shows a number: a missing one is the base
        value (no spell bonus, Blood Gems +1/+1, no Gold spent)."""
        p = self.player()
        out = {}
        if p is None:
            return out
        # What one Blood Gem gives (BACON_BLOODGEMBUFFATKVALUE, +1/+1 base).
        if p["BACON_BLOODGEMBUFFATKVALUE"] not in (0, 1) or \
                p["BACON_BLOODGEMBUFFHEALTHVALUE"] not in (0, 1):
            out["gem_attack"] = p["BACON_BLOODGEMBUFFATKVALUE"] or 1
            out["gem_health"] = p["BACON_BLOODGEMBUFFHEALTHVALUE"] or 1
        # The Tavern spell bonus, which every Tavern spell this player casts
        # and every spell on a minion gets.
        if p["TAVERN_SPELL_ATTACK_INCREASE"]:
            out["spell_attack"] = p["TAVERN_SPELL_ATTACK_INCREASE"]
        if p["TAVERN_SPELL_HEALTH_INCREASE"]:
            out["spell_health"] = p["TAVERN_SPELL_HEALTH_INCREASE"]
        for card_id, (atk_key, hp_key) in PLAYER_ENCHANTS.items():
            sd = self.enchant_numbers(p.id, card_id)
            if sd is None:
                continue
            if atk_key and sd[0]:
                out[atk_key] = sd[0]
            if hp_key and sd[1]:
                out[hp_key] = sd[1]
        if p["NUM_SPELLS_PLAYED_THIS_GAME"]:
            out["spells_cast"] = p["NUM_SPELLS_PLAYED_THIS_GAME"]
        if p["NUM_RESOURCES_SPENT_THIS_GAME"]:
            out["gold_spent"] = p["NUM_RESOURCES_SPENT_THIS_GAME"]
        if self.upper:
            out["upper_hand"] = self.upper
        return out

    def enchant_numbers(self, player_id, card_id):
        """The script numbers of one player enchantment `card_id` on the player
        entity `player_id`, or None when it is not there (the read
        collect_boards.py does)."""
        xs = [x for x in self.game.entities.values()
              if x.card_id == card_id and x["ZONE"] == "PLAY" and x["ATTACHED"] == player_id]
        if not xs:
            return None
        x = max(xs, key=lambda x: x.id)
        return [x[f"TAG_SCRIPT_DATA_NUM_{i}"] for i in range(1, 5)]

    def frozen(self):
        """The Tavern's lock: whether its offers are frozen. The offers' own
        flags are what the game shows (a Freeze click sets them one by one),
        while the Freeze button's tag can lag a toggle behind, so the offers
        are read first and the button only when the Tavern holds nothing."""
        dummy = self.dummy()
        offers = [x for x in self.game.entities.values()
                  if x["CONTROLLER"] == dummy and x["ZONE"] == "PLAY"
                  and x["CARDTYPE"] in SHOP_TYPES]
        if offers:
            return any(x["FROZEN"] for x in offers)
        btn = [x for x in self.game.entities.values()
               if x["CONTROLLER"] == self.me and x["ZONE"] == "PLAY" and x.card_id == FREEZE]
        return bool(btn[0]["TAG_SCRIPT_DATA_NUM_1"]) if btn else False


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
