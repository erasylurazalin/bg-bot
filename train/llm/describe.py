"""One seat's observation, as plain text for an LLM to read.

`bgpy.VecEnv.observe` writes a flat f32 vector: the layout is documented at
the top of `crates/bgcore/src/obs.rs`, and this module decodes it with the
constants `bgpy` exposes, plus the legal-action mask from the same call, into
plain English. Card names come from the environment, hero names from the
environment, card texts from `ref/bg-cards.json` (`text_plain`, the golden
text for a golden card), and hero power, trinket and Dark Gift texts from
`ref/bg-meta.json`.

The action indices are the engine's own (`crates/bgcore/src/action.rs`), so an
answer that names one goes straight back into `VecEnv.step`.

    d = describe.Descriptor(env)
    state, actions = d.describe(obs, mask, seat)
    print(state)
    for index, words in actions:
        print(f"{index}: {words}")

Only reading: nothing here plays the game or changes the environment.
"""

from __future__ import annotations

import json
import re
import tomllib
from pathlib import Path

import bgpy

ROOT = Path(__file__).resolve().parent.parent.parent
REF_CARDS = ROOT / "ref" / "bg-cards.json"
REF_META = ROOT / "ref" / "bg-meta.json"
HEROES_TOML = ROOT / "heroes.toml"
TRINKETS_TOML = ROOT / "trinkets.toml"
RULES_TOML = ROOT / "rules.toml"

# The ten minion types in the engine's own order (`Tribe::EVERY`,
# crates/bgcore/src/cards.rs): a slot's tribe flags are written in this order.
TRIBES = [
    "Beast", "Mech", "Dragon", "Murloc", "Demon",
    "Pirate", "Elemental", "Undead", "Quilboar", "Aberration",
]
# The keywords in the engine's order (`Keywords::ALL`, crates/bgcore/src/minion.rs).
KEYWORDS = [
    "Taunt", "Divine Shield", "Windfury", "Venomous", "Reborn",
    "Magnetic", "Stealth", "Poisonous",
]
# The four combat Secrets of the Secret zone (`secrets::NAMES`, first four).
SECRETS = ["Venomstrike Trap", "Autodefense Matrix", "Pack Tactics", "Redemption"]
# The target kinds, in the observation's one-hot order (`TARGET_KINDS`).
TARGET_KIND_NAMES = [
    "buff", "consume", "spell", "trigger a Rally", "trigger a Battlecry",
    "buff (board or Tavern)", "set stats (board or Tavern)",
]

# Observation layout, from bgpy (all of it mirrors crates/bgcore/src/obs.rs).
SCALARS = bgpy.SCALARS
SLOT = bgpy.SLOT_FEATURES
SHOP = bgpy.SHOP
HAND = bgpy.HAND
BOARD = bgpy.BOARD
DISCOVER = bgpy.DISCOVER
CHOICES = bgpy.CHOICES
SHOP_START = bgpy.SHOP_START
SHOP_SPELL_START = bgpy.SHOP_SPELL_START
HAND_START = bgpy.HAND_START
BOARD_START = bgpy.BOARD_START
DISCOVER_START = bgpy.DISCOVER_START
OWN_END = bgpy.OWN_END
OPP_BLOCK = bgpy.OPP_BLOCK
OPP_SCALARS = bgpy.OPP_SCALARS
PLAYERS = bgpy.PLAYERS
TRINKET_SLOTS = bgpy.TRINKET_SLOTS
TRINKET_CHOICES = bgpy.TRINKET_CHOICES

SLOT_PRESENT = bgpy.SLOT_PRESENT
SLOT_CARD_ID = bgpy.SLOT_CARD_ID
SLOT_IS_SPELL = bgpy.SLOT_IS_SPELL
SLOT_ATTACK = bgpy.SLOT_ATTACK
SLOT_HEALTH = bgpy.SLOT_HEALTH
SLOT_FIGHT_ATTACK = bgpy.SLOT_FIGHT_ATTACK
SLOT_FIGHT_HEALTH = bgpy.SLOT_FIGHT_HEALTH
SLOT_GOLDEN = bgpy.SLOT_GOLDEN
SLOT_TIER = bgpy.SLOT_TIER
SLOT_TRIBES = bgpy.SLOT_TRIBES
SLOT_KEYWORDS = bgpy.SLOT_KEYWORDS
SLOT_MAGNETIZATIONS = bgpy.SLOT_MAGNETIZATIONS
SLOT_VOLUMIZERS = bgpy.SLOT_VOLUMIZERS
SLOT_IMPROVEMENTS = bgpy.SLOT_IMPROVEMENTS
SLOT_EVENTS_LEFT = bgpy.SLOT_EVENTS_LEFT
SLOT_FROM_POOL = bgpy.SLOT_FROM_POOL
SLOT_WINNERS_BREAD = bgpy.SLOT_WINNERS_BREAD
SLOT_TAUGHT_SPELL = bgpy.SLOT_TAUGHT_SPELL
SLOT_LOCKED = bgpy.SLOT_LOCKED
SLOT_ACTIVATE_COST = bgpy.SLOT_ACTIVATE_COST
SLOT_EXTRA_MAGNETIZE = bgpy.SLOT_EXTRA_MAGNETIZE
SLOT_COST = bgpy.SLOT_COST
SLOT_GEM_ATTACK = bgpy.SLOT_GEM_ATTACK
SLOT_GEM_HEALTH = bgpy.SLOT_GEM_HEALTH
SLOT_TEMP_SHIELD = bgpy.SLOT_TEMP_SHIELD
SLOT_DOOMED = bgpy.SLOT_DOOMED
SLOT_BOTH_EFFECTS = bgpy.SLOT_BOTH_EFFECTS
SLOT_LINKED = bgpy.SLOT_LINKED
SLOT_TAVERN_SPELL = bgpy.SLOT_TAVERN_SPELL
SLOT_NO_REWARD = bgpy.SLOT_NO_REWARD
SLOT_ATTACHED = bgpy.SLOT_ATTACHED
SLOT_ATTACHED_GOLDEN = bgpy.SLOT_ATTACHED_GOLDEN
SLOT_GIFTS = bgpy.SLOT_GIFTS
SLOT_HATS = bgpy.SLOT_HATS
SLOT_TICKETS = bgpy.SLOT_TICKETS
SLOT_FROZEN = bgpy.SLOT_FROZEN
SLOT_REANIMATE = bgpy.SLOT_REANIMATE
ATTACHED_SLOTS = bgpy.ATTACHED_SLOTS
GIFT_SLOTS = bgpy.GIFT_SLOTS

SCALAR_GOLD = bgpy.SCALAR_GOLD
SCALAR_TIER = bgpy.SCALAR_TIER
SCALAR_UPGRADE_COST = bgpy.SCALAR_UPGRADE_COST
SCALAR_HEALTH = bgpy.SCALAR_HEALTH
SCALAR_TURN = bgpy.SCALAR_TURN
SCALAR_FROZEN = bgpy.SCALAR_FROZEN
SCALAR_ACTIONS_LEFT = bgpy.SCALAR_ACTIONS_LEFT
SCALAR_ALIVE = bgpy.SCALAR_ALIVE
SCALAR_GHOST = bgpy.SCALAR_GHOST
SCALAR_WON_LAST_COMBAT = bgpy.SCALAR_WON_LAST_COMBAT
SCALAR_LOST_LAST_COMBAT = bgpy.SCALAR_LOST_LAST_COMBAT
SCALAR_GOLD_NEXT_TURN = bgpy.SCALAR_GOLD_NEXT_TURN
SCALAR_EXTRA_MAX_GOLD = bgpy.SCALAR_EXTRA_MAX_GOLD
SCALAR_FREE_ROLLS = bgpy.SCALAR_FREE_ROLLS
SCALAR_NEXT_ROLL_HEALTH = bgpy.SCALAR_NEXT_ROLL_HEALTH
SCALAR_SPELL_DISCOUNT = bgpy.SCALAR_SPELL_DISCOUNT
SCALAR_SHOP_SPELL_COST = bgpy.SCALAR_SHOP_SPELL_COST
SCALAR_SHOP_SPELL_PAY_HEALTH = bgpy.SCALAR_SHOP_SPELL_PAY_HEALTH
SCALAR_SPELL_ATTACK = bgpy.SCALAR_SPELL_ATTACK
SCALAR_SPELL_HEALTH = bgpy.SCALAR_SPELL_HEALTH
SCALAR_VOLUMIZER_ATTACK = bgpy.SCALAR_VOLUMIZER_ATTACK
SCALAR_VOLUMIZER_HEALTH = bgpy.SCALAR_VOLUMIZER_HEALTH
SCALAR_BEETLE_ATTACK = bgpy.SCALAR_BEETLE_ATTACK
SCALAR_BEETLE_HEALTH = bgpy.SCALAR_BEETLE_HEALTH
SCALAR_BEAST_BONUS_ATTACK = bgpy.SCALAR_BEAST_BONUS_ATTACK
SCALAR_BEAST_BONUS_HEALTH = bgpy.SCALAR_BEAST_BONUS_HEALTH
SCALAR_NEXT_TURN_BUFF_ATTACK = bgpy.SCALAR_NEXT_TURN_BUFF_ATTACK
SCALAR_NEXT_TURN_BUFF_HEALTH = bgpy.SCALAR_NEXT_TURN_BUFF_HEALTH
SCALAR_TAVERN_BUFFS = bgpy.SCALAR_TAVERN_BUFFS
TAVERN_BUFF_FILTERS = bgpy.TAVERN_BUFF_FILTERS
SCALAR_LAST_SPELL = bgpy.SCALAR_LAST_SPELL
SCALAR_LOBBY = bgpy.SCALAR_LOBBY
LOBBY_TRIBES = bgpy.LOBBY_TRIBES
SCALAR_GEM_ATTACK = bgpy.SCALAR_GEM_ATTACK
SCALAR_GEM_HEALTH = bgpy.SCALAR_GEM_HEALTH
SCALAR_REFRESH_ATTACK = bgpy.SCALAR_REFRESH_ATTACK
SCALAR_REFRESH_HEALTH = bgpy.SCALAR_REFRESH_HEALTH
SCALAR_UNDEAD_ATTACK = bgpy.SCALAR_UNDEAD_ATTACK
SCALAR_UNDEAD_HEALTH = bgpy.SCALAR_UNDEAD_HEALTH
SCALAR_ELEMENTALS_PLAYED = bgpy.SCALAR_ELEMENTALS_PLAYED
SCALAR_GOLD_SPENT = bgpy.SCALAR_GOLD_SPENT
SCALAR_SPELLS_CAST = bgpy.SCALAR_SPELLS_CAST
SCALAR_LAST_DAMAGE = bgpy.SCALAR_LAST_DAMAGE
SCALAR_DEITY = bgpy.SCALAR_DEITY
SCALAR_DEITY_CTHUN = bgpy.SCALAR_DEITY_CTHUN
SCALAR_DEITY_YSHAARJ = bgpy.SCALAR_DEITY_YSHAARJ
SCALAR_DEITY_ATTACK = bgpy.SCALAR_DEITY_ATTACK
SCALAR_DEITY_HEALTH = bgpy.SCALAR_DEITY_HEALTH
SCALAR_DEITY_GOLDEN = bgpy.SCALAR_DEITY_GOLDEN
SCALAR_PENDING_TARGET = bgpy.SCALAR_PENDING_TARGET
SCALAR_PENDING_TARGET_SHOP = bgpy.SCALAR_PENDING_TARGET_SHOP
SCALAR_PENDING_SHOP_SLOT = bgpy.SCALAR_PENDING_SHOP_SLOT
SCALAR_PENDING_CHOOSE_ONE = bgpy.SCALAR_PENDING_CHOOSE_ONE
SCALAR_PENDING_DISCOVER = bgpy.SCALAR_PENDING_DISCOVER
SCALAR_PENDING_DISCARD = bgpy.SCALAR_PENDING_DISCARD
SCALAR_PENDING_SOURCE = bgpy.SCALAR_PENDING_SOURCE
SCALAR_PENDING_CARD = bgpy.SCALAR_PENDING_CARD
SCALAR_PENDING_RUNS = bgpy.SCALAR_PENDING_RUNS
SCALAR_PENDING_ATTACK = bgpy.SCALAR_PENDING_ATTACK
SCALAR_PENDING_HEALTH = bgpy.SCALAR_PENDING_HEALTH
SCALAR_TARGET_KIND = bgpy.SCALAR_TARGET_KIND
TARGET_KINDS = bgpy.TARGET_KINDS
SCALAR_SHOP_CONSUME_LEFT = bgpy.SCALAR_SHOP_CONSUME_LEFT
SCALAR_SHOP_FISHBAIT = bgpy.SCALAR_SHOP_FISHBAIT
SCALAR_DISCOVER_LOCKED = bgpy.SCALAR_DISCOVER_LOCKED
SCALAR_DISCOVER_DAMAGE = bgpy.SCALAR_DISCOVER_DAMAGE
SCALAR_DISCOVER_DOOMED = bgpy.SCALAR_DISCOVER_DOOMED
SCALAR_DISCOVER_BOTH = bgpy.SCALAR_DISCOVER_BOTH
SCALAR_DISCOVER_QUEUE = bgpy.SCALAR_DISCOVER_QUEUE
SCALAR_CHOOSE_ONE = bgpy.SCALAR_CHOOSE_ONE
CHOOSE_FEATURES = bgpy.CHOOSE_FEATURES
SCALAR_ARMOR = bgpy.SCALAR_ARMOR
SCALAR_HERO = bgpy.SCALAR_HERO
SCALAR_POWER_HERO = bgpy.SCALAR_POWER_HERO
SCALAR_POWER_COST = bgpy.SCALAR_POWER_COST
SCALAR_POWER_USES_LEFT = bgpy.SCALAR_POWER_USES_LEFT
SCALAR_POWER_USED_GAME = bgpy.SCALAR_POWER_USED_GAME
SCALAR_POWER_UNLOCKED = bgpy.SCALAR_POWER_UNLOCKED
SCALAR_POWER_USABLE = bgpy.SCALAR_POWER_USABLE
SCALAR_POWER_PROGRESS = bgpy.SCALAR_POWER_PROGRESS
SCALAR_POWER_LEFT = bgpy.SCALAR_POWER_LEFT
POWER_FEATURES = bgpy.POWER_FEATURES
SCALAR_POWER2_HERO = bgpy.SCALAR_POWER2_HERO
SCALAR_PENDING_HERO = bgpy.SCALAR_PENDING_HERO
SCALAR_PENDING_POWER = bgpy.SCALAR_PENDING_POWER
SCALAR_POWER_OFFER = bgpy.SCALAR_POWER_OFFER
SCALAR_HERO_OFFER = bgpy.SCALAR_HERO_OFFER
SCALAR_TRINKET = bgpy.SCALAR_TRINKET
SCALAR_TRINKET_COUNT = bgpy.SCALAR_TRINKET_COUNT
SCALAR_TRINKET_TRIBE = bgpy.SCALAR_TRINKET_TRIBE
SCALAR_PENDING_TRINKET = bgpy.SCALAR_PENDING_TRINKET
SCALAR_TRINKET_OFFER_SLOT = bgpy.SCALAR_TRINKET_OFFER_SLOT
SCALAR_TRINKET_OPTION = bgpy.SCALAR_TRINKET_OPTION
TRINKET_OPTION_FEATURES = bgpy.TRINKET_OPTION_FEATURES
SCALAR_DARK_GIFT_USES = bgpy.SCALAR_DARK_GIFT_USES
SCALAR_DARK_GIFT_PRESSED = bgpy.SCALAR_DARK_GIFT_PRESSED
SCALAR_DARK_GIFT_READY = bgpy.SCALAR_DARK_GIFT_READY
SCALAR_PARADOX = bgpy.SCALAR_PARADOX
SCALAR_DARK_OFFER = bgpy.SCALAR_DARK_OFFER
SCALAR_FREE_MINION_LEFT = bgpy.SCALAR_FREE_MINION_LEFT
SCALAR_PENDING_WAGER = bgpy.SCALAR_PENDING_WAGER
SCALAR_WAGER_OFFER = bgpy.SCALAR_WAGER_OFFER
SCALAR_WAGER_PICK = bgpy.SCALAR_WAGER_PICK
SCALAR_PENDING_HOLMES = bgpy.SCALAR_PENDING_HOLMES
SCALAR_HOLMES_OFFER = bgpy.SCALAR_HOLMES_OFFER
SCALAR_HOLMES_EMPTY = bgpy.SCALAR_HOLMES_EMPTY
# bgpy does not name the Block 5/6 scalars. They follow the ones it does name,
# in the order crates/bgcore/src/obs.rs lays them out.
SCALAR_ICE_BLOCK = bgpy.SCALAR_HOLMES_EMPTY + 2
SCALAR_IMMUNE = SCALAR_ICE_BLOCK + 1
SCALAR_PRIZE_OFFER_TIER = SCALAR_IMMUNE + 1
SCALAR_TRIPLE_PRIZE = SCALAR_PRIZE_OFFER_TIER + 1
SCALAR_SECRET = SCALAR_TRIPLE_PRIZE + 1
SCALAR_SECRET_OFFER = SCALAR_SECRET + len(SECRETS)
assert SCALAR_SECRET_OFFER + CHOICES == SCALARS, (SCALAR_SECRET_OFFER, SCALARS)

OPP_ALIVE = bgpy.OPP_ALIVE
OPP_HEALTH = bgpy.OPP_HEALTH
OPP_TIER = bgpy.OPP_TIER
OPP_SEEN = bgpy.OPP_SEEN
OPP_TURNS_SINCE = bgpy.OPP_TURNS_SINCE
OPP_NEXT = bgpy.OPP_NEXT
OPP_LAST = bgpy.OPP_LAST
OPP_PLACEMENT = bgpy.OPP_PLACEMENT
OPP_TRIBES = bgpy.OPP_TRIBES
OPP_HERO = bgpy.OPP_HERO
OPP_ARMOR = bgpy.OPP_ARMOR
OPP_TRINKET = bgpy.OPP_TRINKET

# Action layout, mirroring crates/bgcore/src/action.rs.
BUY = 0
BUY_SPELL = BUY + SHOP
SELL = BUY_SPELL + 1
ACTIVATE = SELL + BOARD
PLAY = ACTIVATE + BOARD
TARGET = PLAY + HAND * (BOARD + 1)
TARGET_SHOP = TARGET + BOARD
CHOOSE = TARGET_SHOP + SHOP
DISCARD = CHOOSE + CHOICES
MOVE = DISCARD + HAND
ROLL = MOVE + BOARD * (BOARD - 1)
FREEZE = ROLL + 1
LEVEL = FREEZE + 1
END_TURN = LEVEL + 1
HERO_POWER = END_TURN + 1
HERO_POWER2 = HERO_POWER + 1
CHOOSE_TRINKET = HERO_POWER2 + 1
DARK_GIFT = CHOOSE_TRINKET + TRINKET_CHOICES


def action_at(index):
    """(kind, arg1, arg2) for an engine action index, as action.rs builds it."""
    if index < BUY_SPELL:
        return ("buy", index - BUY, None)
    if index == BUY_SPELL:
        return ("buy_spell", None, None)
    if index < ACTIVATE:
        return ("sell", index - SELL, None)
    if index < PLAY:
        return ("activate", index - ACTIVATE, None)
    if index < TARGET:
        j = index - PLAY
        return ("play", j // (BOARD + 1), j % (BOARD + 1))
    if index < TARGET_SHOP:
        return ("target", index - TARGET, None)
    if index < CHOOSE:
        return ("target_shop", index - TARGET_SHOP, None)
    if index < DISCARD:
        return ("choose", index - CHOOSE, None)
    if index < MOVE:
        return ("discard", index - DISCARD, None)
    if index < ROLL:
        j = index - MOVE
        frm, to = j // (BOARD - 1), j % (BOARD - 1)
        to = to + 1 if to >= frm else to
        return ("move", frm, to)
    if index == ROLL:
        return ("roll", None, None)
    if index == FREEZE:
        return ("freeze", None, None)
    if index == LEVEL:
        return ("level", None, None)
    if index == END_TURN:
        return ("end", None, None)
    if index == HERO_POWER:
        return ("hero_power", None, None)
    if index == HERO_POWER2:
        return ("hero_power2", None, None)
    if index < DARK_GIFT:
        return ("choose_trinket", index - CHOOSE_TRINKET, None)
    if index == DARK_GIFT:
        return ("dark_gift", None, None)
    return ("?", None, None)


def check_layout():
    """Fail loudly if bgpy's action layout is not the one this module mirrors."""
    assert bgpy.END_TURN == END_TURN, (bgpy.END_TURN, END_TURN)
    assert bgpy.HERO_POWER == HERO_POWER
    assert bgpy.HERO_POWER2 == HERO_POWER2
    assert bgpy.N_ACTIONS == DARK_GIFT + 1, (bgpy.N_ACTIONS, DARK_GIFT + 1)


check_layout()


def _n(obs, i):
    """An observation value as an int."""
    return int(round(float(obs[i])))


def _b(obs, i):
    """An observation flag as a bool."""
    return float(obs[i]) != 0.0


def gift_label(name):
    """A Dark Gift's name for a reader: "Gift: X strong" is X, marked strong."""
    bare = name[len("Gift: "):] if name.startswith("Gift: ") else name
    return bare.replace(" strong", " (strong version)")


def strip_counter(text):
    """Drop a printed "(3 left!)" count, or a "(5 Gold left!)" one: the
    observation says the real count, and the text keeps its numbers."""
    return re.sub(r"\s*\(\d+[^)]*left!\)\s*$", "", text)


class Descriptor:
    """Text for one seat: card names from the environment, texts from ref/."""

    def __init__(self, env, ref_cards=REF_CARDS, ref_meta=REF_META,
                 heroes_toml=HEROES_TOML, trinkets_toml=TRINKETS_TOML,
                 rules_toml=RULES_TOML):
        self.card_names = list(env.card_names())
        self.hero_names = list(env.hero_names())
        self.rules = tomllib.loads(Path(rules_toml).read_text())
        self._card_texts, self._all_texts = self._read_card_texts(Path(ref_cards))
        meta = json.loads(Path(ref_meta).read_text())
        self._gift_texts = {}
        for g in meta.get("dark_gifts", []):
            text = (g.get("text_plain") or "").strip()
            if text:
                self._gift_texts.setdefault((g["name"], bool(g.get("is_variant"))), text)
        self._power_meta = {
            p["card_id"]: p for p in meta.get("hero_powers", []) if p.get("card_id")
        }
        self._trinket_meta = {
            t["card_id"]: t for t in meta.get("trinkets", []) if t.get("card_id")
        }
        self.powers = self._read_powers(Path(heroes_toml))
        self.trinkets = self._read_trinkets(Path(trinkets_toml))

    # -- reference data ---------------------------------------------------

    @staticmethod
    def _read_card_texts(path):
        """(name -> {False: printed text, True: golden text}, name -> every text
        a card of that name has). Several cards share a name in the export
        (Blood Gem and its keyword variants), so the second map keeps them."""
        out = {}
        every = {}
        Descriptor._by_id = {}
        for c in json.loads(path.read_text())["cards"]:
            Descriptor._by_id[c["card_id"]] = c
            text = (c.get("text_plain") or "").strip()
            if not text:
                continue
            entry = out.setdefault(c["name"], {False: None, True: None})
            golden = bool(c.get("is_golden"))
            if entry[golden] is None:
                entry[golden] = text
            every.setdefault(c["name"], []).append(text)
        return out, every

    def _read_powers(self, path):
        """The power table in the engine's order: one starting power per hero,
        in hero order, then the later [[power]] entries."""
        raw = tomllib.loads(path.read_text())
        out = [h["power"] for h in raw.get("hero", [])]
        out += [{k: v for k, v in p.items()} for p in raw.get("power", [])]
        return out

    def _read_trinkets(self, path):
        """The trinket table in the engine's order ([[trinket]] blocks). The
        file has multi-line inline tables that `tomllib` rejects, so this
        reads the two keys of each block by hand; the order is the file's."""
        out = []
        for block in path.read_text().split("[[trinket]]")[1:]:
            name = re.search(r'^name = "(.*)"$', block, re.M)
            card = re.search(r'^card_id = "(.*)"$', block, re.M)
            if name and card:
                out.append({"name": name.group(1), "card_id": card.group(1)})
        return out

    # -- the card book ------------------------------------------------------

    _REF_TRIBE = {"BEAST": "Beast", "MECHANICAL": "Mech", "DRAGON": "Dragon",
                  "MURLOC": "Murloc", "DEMON": "Demon", "PIRATE": "Pirate",
                  "ELEMENTAL": "Elemental", "UNDEAD": "Undead",
                  "QUILBOAR": "Quilboar", "ABERRATION": "Aberration"}

    def lobby_tribes(self, obs):
        return [TRIBES[j] for j in range(LOBBY_TRIBES) if _b(obs, SCALAR_LOBBY + j)]

    def card_book(self, obs, ref_cards=REF_CARDS):
        """Every minion and Tavern spell this lobby's pool holds, by tier, with
        its stats, types and text: the in-game card book (bottom right of the
        real client). Only cards the engine has built."""
        lobby = set(self.lobby_tribes(obs))
        built = set(self.card_names)
        minions, spells = {}, {}
        self._book_names = set()
        for c in json.loads(Path(ref_cards).read_text())["cards"]:
            if c.get("is_golden") or not c.get("in_solo_pool") or c["name"] not in built:
                continue
            subsets = {self._REF_TRIBE.get(t, t) for t in c.get("lobby_subsets") or []}
            if subsets and not subsets & lobby:
                continue
            tribes = [self._REF_TRIBE[t] for t in c.get("tribes", []) if t in self._REF_TRIBE]
            is_all = "ALL" in c.get("tribes", [])
            text = strip_counter((c.get("text_plain") or "").strip())
            tier = c.get("tier") or 0
            if c["type"] == "MINION":
                if tribes and not is_all and not set(tribes) & lobby:
                    continue
                kind = "all types" if is_all else ("/".join(tribes) if tribes else "no type")
                line = f"- {c['name']} ({c['attack']}/{c['health']}, {kind})" + (f": {text}" if text else "")
                minions.setdefault(tier, []).append(line)
            else:
                line = f"- {c['name']} ({c.get('cost', 0)} Gold)" + (f": {text}" if text else "")
                spells.setdefault(tier, []).append(line)
            self._book_names.add(c["name"])
        par = self.card_name(_n(obs, SCALAR_PARADOX))
        if par.startswith("Dark Paradox t"):
            c = self._by_id.get("BG36_360t" + par[len("Dark Paradox t"):])
            if c:
                minions.setdefault(c.get("tier") or 0, []).append(
                    f"- {par} ({c['attack']}/{c['health']}, all types): {self.card_text(par)}")
                self._book_names.add(par)
        out = ["## The card book",
               "Every minion and Tavern spell in this game's pool (lobby types: "
               + ", ".join(sorted(lobby)) + "), like the card book in the real game. "
               "Generated cards (tokens, Blood Gems, rewards) are not in it; when a text "
               "you are shown names one, its text is given with the state."]
        for t in sorted(minions):
            out.append(f"### Tier {t} minions")
            out.extend(sorted(minions[t]))
        for t in sorted(spells):
            out.append(f"### Tier {t} Tavern spells")
            out.extend(sorted(spells[t]))
        return "\n".join(out)

    # card names that are also everyday words of the state text or the rules
    _UI_PHRASES = {"Refresh the Tavern", "Refresh", "Freeze", "Level Up", "Gold",
                   "Avenge", "Rally", "Battlecry", "Deathrattle", "Discover", "Magnetic",
                   "Magnetize", "Taunt", "Reborn", "Windfury", "Stealth", "Venomous",
                   "Poisonous", "Divine Shield", "Start of Combat", "Spellcraft",
                   "Choose One", "Immune", "Deity", "Upbeat", "Colossal"}

    def is_pool_card(self, name):
        """Whether the game data has this card in the solo pool (any lobby)."""
        if not hasattr(self, "_pool_names"):
            self._pool_names = {c["name"] for c in self._by_id.values()
                                if c.get("in_solo_pool") and not c.get("is_golden")}
        base = name[:-len(" golden")] if name.endswith(" golden") else name
        return base in self._pool_names or base.startswith("Dark Paradox t")

    def named_cards(self, text):
        """Texts of cards that `text` names but that are not in the card book
        (tokens, generated cards, cards a trinket gives): name -> text."""
        book = getattr(self, "_book_names", set())
        if not hasattr(self, "_name_re"):
            names = sorted((n for n in self._card_texts if len(n) >= 4), key=len, reverse=True)
            self._name_re = re.compile(r"\b(" + "|".join(re.escape(n) for n in names) + r")\b")
        found = {}
        for m in self._name_re.finditer(text):
            n = m.group(1)
            if n in book or n in found or n in self._UI_PHRASES:
                continue
            body = self.card_text(n)
            if body:
                found[n] = strip_counter(body)
        return found

    # -- lookups ----------------------------------------------------------

    def card_name(self, card_id_plus_one):
        """A card name from an observation value (card id + 1, 0 for none)."""
        i = int(card_id_plus_one) - 1
        if 0 <= i < len(self.card_names):
            return self.card_names[i]
        return "?"

    def card_text(self, name, golden=False):
        """The printed text of a card by name (the golden text when golden)."""
        entry = self._card_texts.get(name)
        if entry:
            if entry[golden]:
                return entry[golden]
            if entry[not golden]:
                return entry[not golden]
        # The engine's own names for generated and variant cards.
        if name.startswith("Gift: "):
            return self.gift_text(name)
        if name.startswith("Dark Paradox t"):
            # the engine's "Dark Paradox tN" is the game data's BG36_360tN (golden
            # BG36_360_GtN): its own built-in gift, not the generic card text
            n = name[len("Dark Paradox t"):]
            c = self._by_id.get(f"BG36_360_Gt{n}" if golden else f"BG36_360t{n}")
            if c and (c.get("text_plain") or "").strip():
                return ("This game's Dark Paradox (it has a different Dark Gift, stats and "
                        "Tier each game; this one is the same for every player all game): "
                        + c["text_plain"].strip())
            return self.card_text("Dark Paradox", golden)
        # "Blood Gem (Taunt)" and the like: the base card whose text names it.
        m = re.match(r"^(.*) \(([^)]+)\)$", name)
        if m and m.group(1) in self._all_texts:
            base, tag = m.group(1), m.group(2).lower()
            for text in self._all_texts[base]:
                if tag in text.lower():
                    return text
            return self.card_text(base, golden)
        return ""

    def gift_text(self, name):
        """A Dark Gift's text. The engine names them "Gift: <name>", and the
        per-game stronger versions "<name> strong". A count the text shows as
        a placeholder is left as N: the observation does not carry it."""
        bare = name[len("Gift: "):]
        variant = bare.endswith(" strong")
        key = bare[: -len(" strong")] if variant else bare
        text = (self._gift_texts.get((key, variant))
                or self._gift_texts.get((key, False)) or "")
        text = re.sub(r"\{[0-9]\}", "N", text)
        text = re.sub(r"\s*\(N\)\s*$", "", text)
        return strip_counter(text)

    def hero_name(self, index_plus_one):
        i = int(index_plus_one) - 1
        if 0 <= i < len(self.hero_names):
            return self.hero_names[i]
        return "?"

    def power_text(self, index_plus_one):
        """(name, text) of a hero power by its table index + 1."""
        i = int(index_plus_one) - 1
        if i < 0 or i >= len(self.powers):
            return ("?", "")
        p = self.powers[i]
        meta = self._power_meta.get(p.get("card_id", ""))
        text = (meta or {}).get("text_plain") or ""
        return (p.get("name", "?"), text.strip())

    def trinket_name(self, index_plus_one):
        i = int(index_plus_one) - 1
        if i < 0 or i >= len(self.trinkets):
            return ("?", "")
        t = self.trinkets[i]
        meta = self._trinket_meta.get(t.get("card_id", ""))
        return (t.get("name", "?"), ((meta or {}).get("text_plain") or "").strip())

    # -- card slots -------------------------------------------------------

    def slot(self, obs, start, k):
        o = start + k * SLOT
        return obs[o:o + SLOT]

    def present(self, f):
        return float(f[SLOT_PRESENT]) != 0.0

    def card_words(self, f, text=True, fight_stats=True, from_pool_note=True,
                   activate_note=True):
        """One card in a slot, as one line.

        `from_pool_note` is off for a board that was only ever seen in a fight
        (the observation does not track pool copies there, so every card would
        read as a token), and `activate_note` is off where the card cannot be
        activated (the Tavern, a Discover offer)."""
        name = self.card_name(f[SLOT_CARD_ID])
        golden = float(f[SLOT_GOLDEN]) != 0.0
        tier = _n(f, SLOT_TIER)
        if float(f[SLOT_IS_SPELL]) != 0.0:
            cost = _n(f, SLOT_COST)
            kind = "Tavern spell" if float(f[SLOT_TAVERN_SPELL]) else "spell"
            body = self.card_text(name, golden)
            out = f"{name} ({kind}, tier {tier}, {cost} Gold)"
            return out + (f": {body}" if body else "")
        parts = [f"tier {tier}"]
        head = f"{name} {_n(f, SLOT_ATTACK)}/{_n(f, SLOT_HEALTH)}"
        if golden:
            head += " golden"
        tribes = [TRIBES[j] for j in range(len(TRIBES)) if float(f[SLOT_TRIBES + j]) != 0.0]
        kws = [
            KEYWORDS[j] for j in range(len(KEYWORDS))
            if float(f[SLOT_KEYWORDS + j]) != 0.0
        ]
        if len(tribes) == len(TRIBES):
            parts.append("all types")
        elif tribes:
            parts.append("/".join(tribes))
        else:
            parts.append("neutral")
        if kws:
            parts.append(", ".join(kws))
        head += " (" + ", ".join(parts) + ")"
        notes = []
        fa, fh = _n(f, SLOT_FIGHT_ATTACK), _n(f, SLOT_FIGHT_HEALTH)
        if fight_stats and (fa, fh) != (_n(f, SLOT_ATTACK), _n(f, SLOT_HEALTH)):
            notes.append(f"fights as {fa}/{fh}")
        if activate_note and _n(f, SLOT_ACTIVATE_COST):
            notes.append(f"Activate {_n(f, SLOT_ACTIVATE_COST)} Gold")
        if _n(f, SLOT_LOCKED):
            notes.append("locked in hand")
        if _n(f, SLOT_DOOMED):
            notes.append("dies right after it is played this turn")
        if _n(f, SLOT_BOTH_EFFECTS):
            notes.append("both Choose One effects")
        if _n(f, SLOT_NO_REWARD):
            notes.append("golden gives no Triple Reward")
        if _n(f, SLOT_MAGNETIZATIONS):
            notes.append(f"magnetized {_n(f, SLOT_MAGNETIZATIONS)} times")
        if _n(f, SLOT_EXTRA_MAGNETIZE):
            notes.append("the next Magnetization happens again")
        if _n(f, SLOT_HATS):
            notes.append(f"{_n(f, SLOT_HATS)} hats")
        if _n(f, SLOT_WINNERS_BREAD):
            notes.append(f"{_n(f, SLOT_WINNERS_BREAD)} Winner's Bread stacks")
        if _n(f, SLOT_IMPROVEMENTS) or _n(f, SLOT_EVENTS_LEFT):
            notes.append(f"improved {_n(f, SLOT_IMPROVEMENTS)} times")
        if _n(f, SLOT_TEMP_SHIELD):
            notes.append("Divine Shield only until next turn")
        if _n(f, SLOT_REANIMATE):
            notes.append("resummons itself at Start of Combat")
        if _n(f, SLOT_TICKETS):
            notes.append("carries a Darkmoon Ticket")
        if _n(f, SLOT_FROZEN):
            notes.append("frozen on its own")
        if _n(f, SLOT_LINKED):
            notes.append("linked: playing one discards its partners")
        # SLOT_FROM_POOL only says whether this copy holds one of the pool's
        # copies (a Discovered or generated copy of a real card holds none). Only
        # a card that is no pool card at all is a token.
        if (from_pool_note and not _n(f, SLOT_FROM_POOL) and float(f[SLOT_IS_SPELL]) == 0.0
                and not self.is_pool_card(name)):
            notes.append("a token, not a pool card")
        events = _n(f, SLOT_EVENTS_LEFT)
        if not notes:
            return head + self._text_suffix(name, golden, text, events)
        return head + " [" + "; ".join(notes) + "]" + self._text_suffix(
            name, golden, text, events)

    def _text_suffix(self, name, golden, want_text, events_left=0):
        """": <card text>" for a card line, or "" when text is off or the card
        has none. The real event count replaces the printed one."""
        if not want_text:
            return ""
        body = strip_counter(self.card_text(name, golden))
        if events_left:
            body = f"{body} ({events_left} left!)" if body else f"({events_left} left!)"
        return f": {body}" if body else ""

    def attached_words(self, f):
        """The cards Magnetized into a minion and the Dark Gifts on it."""
        out = []
        for j in range(ATTACHED_SLOTS):
            cid = _n(f, SLOT_ATTACHED + j)
            if not cid:
                break
            name = self.card_name(cid)
            golden = float(f[SLOT_ATTACHED_GOLDEN + j]) != 0.0
            out.append(("Magnetized into it: ", name + (" (golden)" if golden else ""),
                        self.card_text(name, golden)))
        for j in range(GIFT_SLOTS):
            cid = _n(f, SLOT_GIFTS + j)
            if not cid:
                break
            name = self.card_name(cid)
            out.append(("Dark Gift: ", gift_label(name), self.gift_text(name)))
        return out

    def board_lines(self, obs, start, count, indent="  ", own=True):
        """A board in order; `own` is off for an opponent's seen board, where a
        card's pool backing and Activate cost are not the player's business."""
        lines = []
        n = 0
        for k in range(count):
            f = self.slot(obs, start, k)
            if not self.present(f):
                continue
            n += 1
            lines.append(f"{indent}{n}. "
                         f"{self.card_words(f, from_pool_note=own, activate_note=own)}")
            for prefix, name, text in self.attached_words(f):
                lines.append(f"{indent}   {prefix}{name}" + (f": {text}" if text else ""))
        return lines

    def shop_lines(self, obs):
        lines = []
        for k in range(SHOP):
            f = self.slot(obs, SHOP_START, k)
            if not self.present(f):
                continue
            lines.append(f"  Tavern slot {k + 1}: "
                         f"{self.card_words(f, activate_note=False)}")
        f = self.slot(obs, SHOP_SPELL_START, 0)
        if self.present(f):
            cost = _n(obs, SCALAR_SHOP_SPELL_COST)
            pay_health = _b(obs, SCALAR_SHOP_SPELL_PAY_HEALTH)
            price = f"paid with {cost} Health, not Gold" if pay_health else f"{cost} Gold"
            body = self.card_text(self.card_name(f[SLOT_CARD_ID]))
            lines.append(
                f"  Tavern spell slot: {self.card_name(f[SLOT_CARD_ID])} ({price})"
                + (f": {body}" if body else "")
            )
        return lines

    def hand_lines(self, obs):
        lines = []
        n = 0
        for k in range(HAND):
            f = self.slot(obs, HAND_START, k)
            if not self.present(f):
                continue
            n += 1
            lines.append(f"  {n}. {self.card_words(f)}")
            for prefix, name, text in self.attached_words(f):
                lines.append(f"     {prefix}{name}" + (f": {text}" if text else ""))
        return lines, n

    def discover_options(self, obs):
        """The open Discover's options, one line each."""
        out = []
        dark = _b(obs, SCALAR_DARK_OFFER)
        for k in range(DISCOVER):
            f = self.slot(obs, DISCOVER_START, k)
            if not self.present(f):
                continue
            head = self.card_words(f, text=False, fight_stats=False,
                                  activate_note=False)
            for j in range(GIFT_SLOTS):
                cid = _n(f, SLOT_GIFTS + j)
                if not cid:
                    continue
                gname = self.card_name(cid)
                gtext = self.gift_text(gname).rstrip(".")
                head += (f" with Dark Gift {gift_label(gname)}"
                         + (f" ({gtext})" if gtext else ""))
            name = self.card_name(f[SLOT_CARD_ID])
            body = self.card_text(name, float(f[SLOT_GOLDEN]) != 0.0)
            if body:
                head += f". Text: {strip_counter(body)}"
            out.append(head)
        return out

    def choose_one_options(self, obs):
        """The open Choose One's two options, from the choice scalars."""
        out = []
        for k in range(2):
            base = SCALAR_CHOOSE_ONE + k * CHOOSE_FEATURES
            kind = None
            for j, label in enumerate(["buff a minion", "spell stat bonus", "spell target",
                                       "spell effect"]):
                if _b(obs, base + j):
                    kind = label
            a, h = _n(obs, base + 4), _n(obs, base + 5)
            words = kind or "option"
            if a or h:
                words += f" ({a:+d}/{h:+d})"
            out.append(words)
        return out

    def hero_offer_options(self, obs):
        out = []
        for k in range(DISCOVER):
            h = _n(obs, SCALAR_HERO_OFFER + k)
            if h:
                out.append(self.hero_name(h))
        return out

    def power_offer_options(self, obs):
        out = []
        for k in range(CHOICES):
            p = _n(obs, SCALAR_POWER_OFFER + k)
            if p:
                name, text = self.power_text(p)
                out.append(f"{name} ({text})" if text else name)
        return out

    def trinket_offer_options(self, obs):
        out = []
        for k in range(TRINKET_CHOICES):
            t = _n(obs, SCALAR_TRINKET_OPTION + k * TRINKET_OPTION_FEATURES)
            if not t:
                continue
            price = _n(obs, SCALAR_TRINKET_OPTION + k * TRINKET_OPTION_FEATURES + 1)
            name, text = self.trinket_name(t)
            out.append((name, text, price))
        return out

    # -- the text ---------------------------------------------------------

    def max_gold(self, obs):
        """This turn's unmodified Gold (rules.toml: 3 on turn 1, then 1 more a
        turn, capped) plus whatever a card has added to it."""
        turn = max(1, _n(obs, SCALAR_TURN))
        base = min(self.rules.get("max_gold", 10),
                   self.rules.get("first_turn_gold", 3) + turn - 1)
        return base + _n(obs, SCALAR_EXTRA_MAX_GOLD)

    def power_line(self, obs, base, label):
        of = _n(obs, base)
        if not of:
            return None
        name, text = self.power_text(of)
        cost = _n(obs, base + 1)
        uses = _n(obs, base + 2)
        used = _n(obs, base + 3)
        unlocked = _b(obs, base + 4)
        usable = _b(obs, base + 5)
        progress = _n(obs, base + 6)
        left = _n(obs, base + 7)
        passive = bool(self.powers[min(of, len(self.powers)) - 1].get("passive", False))
        bits = [f"{label}: {name}"]
        if passive:
            bits.append("passive, no button")
        else:
            bits.append("free" if cost == 0 else f"{cost} Gold")
            if uses:
                bits.append(f"{uses} use{'s' if uses != 1 else ''} left this turn")
        if used:
            bits.append(f"used {used} time{'s' if used != 1 else ''} this game")
        if not unlocked:
            bits.append("locked")
        if usable:
            bits.append("you can use it now")
        if progress or left:
            bits.append(f"count {progress}, {left} to go")
        if text:
            bits.append(f"text: {text}")
        return ", ".join([bits[0]]) + " (" + "; ".join(bits[1:]) + ")"

    def trinket_line(self, obs):
        parts = []
        for slot, label in enumerate(["Lesser trinket", "Greater trinket",
                                     "Hero power trinket"]):
            of = _n(obs, SCALAR_TRINKET + slot)
            if not of:
                parts.append(f"{label}: none")
                continue
            name, text = self.trinket_name(of)
            count = _n(obs, SCALAR_TRINKET_COUNT + slot)
            tribe = _n(obs, SCALAR_TRINKET_TRIBE + slot)
            extra = f", counter {count}" if count else ""
            if tribe:
                extra += f", locked to {TRIBES[tribe - 1]}"
            parts.append(f"{label}: {name}{extra}" + (f" ({text})" if text else ""))
        gifts = []
        for k in range(BOARD):
            f = self.slot(obs, BOARD_START, k)
            if not self.present(f):
                continue
            for j in range(GIFT_SLOTS):
                cid = _n(f, SLOT_GIFTS + j)
                if not cid:
                    break
                name = self.card_name(cid)
                gifts.append(f"{gift_label(name)} on {self.card_name(f[SLOT_CARD_ID])}"
                             + (f" ({self.gift_text(name)})" if self.gift_text(name) else ""))
        if gifts:
            parts.append("Dark Gifts held: " + "; ".join(gifts))
        return "; ".join(parts)

    def choice_lines(self, obs, mask=None):
        """Whatever choice is open, and what each answer means (for Choose(k)).

        With the legal-action `mask`, a target choice says where it can point
        (the mask is the truth about which slots are offered)."""
        lines = []
        words = []
        if _b(obs, SCALAR_PENDING_HERO):
            names = self.hero_offer_options(obs)
            lines.append("CHOICE OPEN: pick your hero. " + ", ".join(
                f"option {k + 1}: {n}" for k, n in enumerate(names)))
            words = [f"pick hero {n}" for n in names]
        elif _b(obs, SCALAR_PENDING_TRINKET):
            opts = self.trinket_offer_options(obs)
            slot = _n(obs, SCALAR_TRINKET_OFFER_SLOT)
            which = {1: "lesser", 2: "greater", 3: "hero power"}.get(slot, "?")
            lines.append(f"CHOICE OPEN: pick your {which} trinket. " + "; ".join(
                f"option {k + 1}: {n} for {p} Gold" + (f" ({t})" if t else "")
                for k, (n, t, p) in enumerate(opts)))
        elif _b(obs, SCALAR_PENDING_TARGET) or _b(obs, SCALAR_PENDING_TARGET_SHOP):
            card = self.pending_card(obs) or "the effect that asked"
            kind = TARGET_KIND_NAMES[0]
            for j in range(TARGET_KINDS):
                if _b(obs, SCALAR_TARGET_KIND + j):
                    kind = TARGET_KIND_NAMES[j]
            a, h = _n(obs, SCALAR_PENDING_ATTACK), _n(obs, SCALAR_PENDING_HEALTH)
            stats = f" ({a:+d}/{h:+d})" if (a or h) else ""
            to_shop = _b(obs, SCALAR_PENDING_TARGET_SHOP)
            if mask is not None:
                to_shop = any(mask[TARGET_SHOP:TARGET_SHOP + SHOP])
            where = "your board or the Tavern" if to_shop else "your board"
            runs = _n(obs, SCALAR_PENDING_RUNS)
            lines.append(
                f"CHOICE OPEN: pick a target in {where} for {card}: a {kind}{stats}"
                + (f", {runs} times" if runs > 1 else "")
            )
        elif _b(obs, SCALAR_PENDING_SHOP_SLOT):
            card = self.pending_card(obs) or "the effect that asked"
            if _b(obs, SCALAR_SHOP_FISHBAIT):
                what = "put a Fishbait into that Tavern slot"
            else:
                what = (f"consume that Tavern minion "
                        f"({_n(obs, SCALAR_SHOP_CONSUME_LEFT)} more after it)")
            lines.append(f"CHOICE OPEN: pick a Tavern slot for {card}: {what}")
        elif _b(obs, SCALAR_PENDING_DISCARD):
            lines.append("CHOICE OPEN: discard a card from your hand")
        elif _b(obs, SCALAR_PENDING_CHOOSE_ONE):
            opts = self.choose_one_options(obs)
            lines.append("CHOICE OPEN: Choose One. " + "; ".join(
                f"option {k + 1}: {o}" for k, o in enumerate(opts)))
            words = [f"choose option {k + 1}: {o}" for k, o in enumerate(opts)]
        elif _b(obs, SCALAR_PENDING_DISCOVER):
            opts = self.discover_options(obs)
            notes = []
            if _b(obs, SCALAR_DISCOVER_LOCKED):
                notes.append("the card stays locked in your hand for a turn")
            if _b(obs, SCALAR_DISCOVER_DAMAGE):
                notes.append("your hero takes damage equal to its tier")
            if _b(obs, SCALAR_DISCOVER_DOOMED):
                notes.append("it dies right after it is played this turn")
            if _b(obs, SCALAR_DISCOVER_BOTH):
                notes.append("a Choose One card with both effects")
            if _n(obs, SCALAR_DISCOVER_QUEUE):
                notes.append(f"{_n(obs, SCALAR_DISCOVER_QUEUE)} more Discovers follow")
            if _b(obs, SCALAR_DARK_OFFER):
                notes.append("a Dark Gift offer: the gift comes with the minion")
            tier = _n(obs, SCALAR_PRIZE_OFFER_TIER)
            if tier:
                notes.append(f"a tier {tier} Prize")
            lines.append(
                "CHOICE OPEN: Discover, pick one: "
                + "; ".join(f"option {k + 1}: {o}" for k, o in enumerate(opts))
                + (" [" + "; ".join(notes) + "]" if notes else "")
            )
            words = [f"pick {o}" for o in opts]
        elif _b(obs, SCALAR_PENDING_POWER):
            opts = self.power_offer_options(obs)
            lines.append("CHOICE OPEN: pick a hero power. " + "; ".join(
                f"option {k + 1}: {o}" for k, o in enumerate(opts)))
            words = [f"pick the hero power {o}" for o in opts]
        elif any(_b(obs, SCALAR_SECRET_OFFER + k) for k in range(CHOICES)):
            opts = []
            for k in range(CHOICES):
                v = _n(obs, SCALAR_SECRET_OFFER + k)
                if v:
                    opts.append(SECRETS[v - 1] if 0 < v <= len(SECRETS) else f"Secret {v}")
            lines.append("CHOICE OPEN: pick a Secret. " + "; ".join(
                f"option {k + 1}: {o}" for k, o in enumerate(opts)))
            words = [f"pick the Secret {o}" for o in opts]
        elif _b(obs, SCALAR_PENDING_WAGER):
            seats = [_n(obs, SCALAR_WAGER_OFFER), _n(obs, SCALAR_WAGER_OFFER + 1)]
            lines.append("CHOICE OPEN: Friendly Wager, pick the player who will win: "
                         + "; ".join(f"option {k + 1}: seat {s}" for k, s in enumerate(seats)))
            words = [f"bet on seat {s}" for s in seats]
        elif _b(obs, SCALAR_PENDING_HOLMES):
            opts = []
            for k in range(2):
                if _b(obs, SCALAR_HOLMES_EMPTY + k):
                    opts.append("an empty board")
                else:
                    opts.append(self.card_name(_n(obs, SCALAR_HOLMES_OFFER + k))
                                if _n(obs, SCALAR_HOLMES_OFFER + k) else "an unknown card")
            lines.append("CHOICE OPEN: Detective for Hire, guess the card: "
                         + "; ".join(f"option {k + 1}: {o}" for k, o in enumerate(opts)))
            words = [f"guess {o}" for o in opts]
        return lines, words

    def has_choice(self, obs, mask=None):
        """Whether some choice (a target, a Discover, a pick) is open: while one
        is, only answers to it are legal."""
        return bool(self.choice_lines(obs, mask)[0])

    def state(self, obs, seat=None, mask=None):
        """The state text: everything this seat can see."""
        turn = _n(obs, SCALAR_TURN)
        out = []
        out.append(f"TURN {turn}. {_n(obs, SCALAR_ALIVE)} players are still alive.")
        hero = self.hero_name(_n(obs, SCALAR_HERO)) if _n(obs, SCALAR_HERO) else "unknown"
        out.append(f"You are {hero}"
                   + (f" (seat {seat})" if seat is not None else "")
                   + ".")
        gold = _n(obs, SCALAR_GOLD)
        line = (f"Health {_n(obs, SCALAR_HEALTH)}, armor {_n(obs, SCALAR_ARMOR)}. "
                f"Gold {gold} of {self.max_gold(obs)} this turn")
        if _n(obs, SCALAR_GOLD_NEXT_TURN):
            line += f", plus {_n(obs, SCALAR_GOLD_NEXT_TURN)} already queued for next turn"
        line += f". Actions left this turn: {_n(obs, SCALAR_ACTIONS_LEFT)}."
        out.append(line)
        tier = _n(obs, SCALAR_TIER)
        if tier < bgpy.MAX_TIER:
            out.append(f"Tavern tier {tier}; upgrading to tier {tier + 1} costs "
                       f"{_n(obs, SCALAR_UPGRADE_COST)} Gold.")
        else:
            out.append(f"Tavern tier {tier} (the highest).")

        tribes = [TRIBES[j] for j in range(LOBBY_TRIBES) if _b(obs, SCALAR_LOBBY + j)]
        out.append("Lobby types: " + (", ".join(tribes) if tribes else "none") + ".")

        # Hero powers.
        for base, label in ((SCALAR_POWER_HERO, "Hero power"),
                            (SCALAR_POWER2_HERO, "Second hero power")):
            line = self.power_line(obs, base, label)
            if line:
                out.append(line)

        # Trinkets and Dark Gifts.
        out.append("Trinkets: " + self.trinket_line(obs) + ".")

        # Modifiers worth knowing.
        mods = []
        pairs = [
            ("Tavern spell bonus", SCALAR_SPELL_ATTACK),
            ("Blood Gems give", SCALAR_GEM_ATTACK),
            ("Undead bonus", SCALAR_UNDEAD_ATTACK),
            ("Beetle bonus", SCALAR_BEETLE_ATTACK),
            ("Volumizer bonus", SCALAR_VOLUMIZER_ATTACK),
            ("Beast bonus until next turn", SCALAR_BEAST_BONUS_ATTACK),
            ("board buff queued for next turn", SCALAR_NEXT_TURN_BUFF_ATTACK),
            ("each Refresh gives one Tavern minion", SCALAR_REFRESH_ATTACK),
        ]
        for label, at in pairs:
            a, h = _n(obs, at), _n(obs, at + 1)
            if a or h:
                mods.append(f"{label} {a:+d}/{h:+d}")
        if _n(obs, SCALAR_FREE_ROLLS):
            mods.append(f"{_n(obs, SCALAR_FREE_ROLLS)} free Refreshes left")
        if _b(obs, SCALAR_NEXT_ROLL_HEALTH):
            mods.append("the next Refresh costs Health, not Gold")
        if _n(obs, SCALAR_SPELL_DISCOUNT):
            mods.append(f"{_n(obs, SCALAR_SPELL_DISCOUNT)} Gold off the next Tavern spell")
        if _n(obs, SCALAR_FREE_MINION_LEFT):
            mods.append("the next minion you buy this turn is free")
        if _n(obs, SCALAR_LAST_SPELL):
            mods.append(f"your last spell cast was {self.card_name(_n(obs, SCALAR_LAST_SPELL))}")
        if _n(obs, SCALAR_SPELLS_CAST) or _n(obs, SCALAR_GOLD_SPENT):
            mods.append(f"{_n(obs, SCALAR_SPELLS_CAST)} Tavern spells cast this game, "
                        f"{_n(obs, SCALAR_GOLD_SPENT)} Gold spent this turn")
        if _n(obs, SCALAR_ELEMENTALS_PLAYED):
            mods.append(f"{_n(obs, SCALAR_ELEMENTALS_PLAYED)} Elementals played this turn")
        for j in range(TAVERN_BUFF_FILTERS):
            base = SCALAR_TAVERN_BUFFS + j * 4
            a, h = _n(obs, base), _n(obs, base + 1)
            ta, th = _n(obs, base + 2), _n(obs, base + 3)
            if not (a or h or ta or th):
                continue
            who = "Tavern minions" if j == 0 else TRIBES[j - 1] + "s in the Tavern"
            if (a, h) != (0, 0):
                mods.append(f"{who} have {a:+d}/{h:+d} this game")
            if (ta, th) != (0, 0):
                mods.append(f"{who} have {ta:+d}/{th:+d} this turn")
        if _b(obs, SCALAR_DEITY):
            god = "C'Thun" if _b(obs, SCALAR_DEITY_CTHUN) else "Y'Shaarj"
            mods.append(f"your Deity is {god} {_n(obs, SCALAR_DEITY_ATTACK)}/"
                        f"{_n(obs, SCALAR_DEITY_HEALTH)}"
                        + (" golden" if _b(obs, SCALAR_DEITY_GOLDEN) else ""))
        if _n(obs, SCALAR_DARK_GIFT_USES):
            mods.append(f"{_n(obs, SCALAR_DARK_GIFT_USES)} Dark Discovery uses left this game"
                        + (", already pressed this turn" if _b(obs, SCALAR_DARK_GIFT_PRESSED)
                           else ""))
        if _b(obs, SCALAR_ICE_BLOCK):
            mods.append("you hold an unspent Ice Block")
        if _b(obs, SCALAR_IMMUNE):
            mods.append("your hero is immune this turn")
        secrets = [SECRETS[j] for j in range(len(SECRETS)) if _b(obs, SCALAR_SECRET + j)]
        if secrets:
            mods.append("your Secrets: " + ", ".join(secrets))
        if _b(obs, SCALAR_TRIPLE_PRIZE):
            mods.append("Triple Rewards are Triple Prizes this game")
        if mods:
            out.append("Modifiers: " + "; ".join(mods) + ".")

        # The Tavern.
        frozen = "frozen" if _b(obs, SCALAR_FROZEN) else "not frozen"
        out.append("")
        out.append(f"THE TAVERN ({frozen}).")
        out.extend(self.shop_lines(obs))

        # Hand.
        out.append("")
        hand = self.hand_lines(obs)
        out.append(f"YOUR HAND ({hand[1]} of {HAND}):")
        out.extend(hand[0])

        # Board.
        out.append("")
        count = sum(1 for k in range(BOARD) if self.present(self.slot(obs, BOARD_START, k)))
        out.append(f"YOUR BOARD ({count} of {BOARD}, left to right, position 1 is the leftmost):")
        out.extend(self.board_lines(obs, BOARD_START, BOARD))

        # Open choices.
        choice, _words = self.choice_lines(obs, mask)
        if choice:
            out.append("")
            out.extend(choice)

        # Opponents.
        out.append("")
        out.append("OPPONENTS (in seat order after you):")
        for k in range(1, PLAYERS):
            base = OWN_END + (k - 1) * OPP_BLOCK
            seat_no = (seat + k) % PLAYERS if seat is not None else k
            who = self.hero_name(_n(obs, base + OPP_HERO)) if _n(obs, base + OPP_HERO) else "no hero"
            bits = [f"  Opponent {k} (seat {seat_no}): {who}"]
            health = max(0, _n(obs, base + OPP_HEALTH))
            bits.append(f"Health {health}, armor {_n(obs, base + OPP_ARMOR)}, "
                        f"tier {_n(obs, base + OPP_TIER)}")
            if not _b(obs, base + OPP_ALIVE):
                bits.append(f"KNOCKED OUT, finished {_n(obs, base + OPP_PLACEMENT)}")
            elif _b(obs, base + OPP_NEXT):
                bits.append("you fight them THIS TURN"
                            + (" (their ghost)" if _b(obs, SCALAR_GHOST) else ""))
            elif _b(obs, base + OPP_LAST):
                bits.append("they were your last fight")
            trinkets = []
            for slot in range(TRINKET_SLOTS):
                of = _n(obs, base + OPP_TRINKET + slot)
                if of:
                    trinkets.append(self.trinket_name(of)[0])
            if trinkets:
                bits.append("trinkets: " + ", ".join(trinkets))
            out.append(bits[0] + ": " + ", ".join(bits[1:]) + ".")
            if _b(obs, base + OPP_SEEN):
                since = _n(obs, base + OPP_TURNS_SINCE)
                when = ("this turn" if since == 0 else
                        f"{since} turn{'s' if since != 1 else ''} ago")
                board = self.board_lines(obs, base + OPP_SCALARS, BOARD, indent="     ",
                                         own=False)
                out.append(f"     board you last saw, {when}: "
                           + ("" if board else "none left"))
                out.extend(board)
            else:
                out.append("     you have not fought them yet.")

        # Last combat.
        if _b(obs, SCALAR_WON_LAST_COMBAT):
            out.append("")
            out.append("Your last fight was a win.")
        elif _b(obs, SCALAR_LOST_LAST_COMBAT):
            out.append("")
            out.append("Your last fight was a loss.")
        elif turn > 1:
            out.append("")
            out.append("Your last fight was a tie.")
        return "\n".join(out)

    def actions(self, obs, mask):
        """Every legal action as (engine index, plain words), in index order."""
        rows = []
        for index in range(len(mask)):
            if not mask[index]:
                continue
            rows.append((index, self.action_words(obs, index)))
        return rows

    def action_words(self, obs, index):
        kind, a, b = action_at(index)
        if kind == "buy":
            f = self.slot(obs, SHOP_START, a)
            name = self.card_name(f[SLOT_CARD_ID])
            if float(f[SLOT_IS_SPELL]) != 0.0:
                return f"buy the Tavern spell {name} ({_n(f, SLOT_COST)} Gold)"
            return (f"buy {name} ({_n(f, SLOT_ATTACK)}/{_n(f, SLOT_HEALTH)}) "
                    f"from Tavern slot {a + 1} for {_n(f, SLOT_COST)} Gold")
        if kind == "buy_spell":
            name = self.card_name(self.slot(obs, SHOP_SPELL_START, 0)[SLOT_CARD_ID])
            if _b(obs, SCALAR_SHOP_SPELL_PAY_HEALTH):
                return (f"buy the Tavern spell {name} for "
                        f"{_n(obs, SCALAR_SHOP_SPELL_COST)} Health instead of Gold")
            return f"buy the Tavern spell {name} ({_n(obs, SCALAR_SHOP_SPELL_COST)} Gold)"
        if kind == "sell":
            f = self.slot(obs, BOARD_START, a)
            return (f"sell {self.card_name(f[SLOT_CARD_ID])} from board position {a + 1} "
                    f"for 1 Gold")
        if kind == "activate":
            f = self.slot(obs, BOARD_START, a)
            cost = _n(f, SLOT_ACTIVATE_COST)
            return (f"use {self.card_name(f[SLOT_CARD_ID])}'s Activate at board position "
                    f"{a + 1} for {cost} Gold")
        if kind == "play":
            f = self.slot(obs, HAND_START, a)
            name = self.card_name(f[SLOT_CARD_ID])
            if float(f[SLOT_IS_SPELL]) != 0.0:
                return f"cast {name} from hand slot {a + 1}"
            count = sum(1 for k in range(BOARD)
                        if self.present(self.slot(obs, BOARD_START, k)))
            # Position 8 is the insertion point after the last of the 7 slots.
            end = " (the right end)" if b + 1 > BOARD else ""
            words = (f"play {name} from hand slot {a + 1} to board position {b + 1}{end}")
            # A Magnetic card played here merges into the minion that would sit
            # to its right (`Game::can_magnetize`, Mech onto Mech for all but a
            # few cards).
            if float(f[SLOT_KEYWORDS + KEYWORDS.index("Magnetic")]) != 0.0 and b < count:
                right = self.slot(obs, BOARD_START, b)
                mech = float(right[SLOT_TRIBES + TRIBES.index("Mech")]) != 0.0
                if self.present(right) and mech:
                    words += (f" (Magnetize: it merges into "
                              f"{self.card_name(right[SLOT_CARD_ID])} instead)")
            return words
        if kind == "target":
            f = self.slot(obs, BOARD_START, a)
            return (f"pick your minion at board position {a + 1} "
                    f"({self.card_name(f[SLOT_CARD_ID])}) as the target"
                    + self._target_suffix(obs))
        if kind == "target_shop":
            f = self.slot(obs, SHOP_START, a)
            if not self.present(f):
                f = self.slot(obs, SHOP_SPELL_START, 0)
            return (f"pick Tavern slot {a + 1} ({self.card_name(f[SLOT_CARD_ID])}) "
                    "as the target" + self._target_suffix(obs))
        if kind == "choose":
            _lines, words = self.choice_lines(obs)
            if a < len(words):
                return words[a]
            return f"choose option {a + 1}"
        if kind == "discard":
            f = self.slot(obs, HAND_START, a)
            return (f"discard {self.card_name(f[SLOT_CARD_ID])} from hand slot {a + 1}")
        if kind == "move":
            f = self.slot(obs, BOARD_START, a)
            return (f"move {self.card_name(f[SLOT_CARD_ID])} from board position {a + 1} "
                    f"to position {b + 1}")
        if kind == "roll":
            if _n(obs, SCALAR_FREE_ROLLS):
                return "Refresh the Tavern (free)"
            if _b(obs, SCALAR_NEXT_ROLL_HEALTH):
                return "Refresh the Tavern (costs Health, not Gold)"
            return f"Refresh the Tavern ({self.rules.get('roll_cost', 1)} Gold)"
        if kind == "freeze":
            if _b(obs, SCALAR_FROZEN):
                return "unfreeze the Tavern"
            return "freeze the Tavern (free, keeps these cards for next turn)"
        if kind == "level":
            tier = _n(obs, SCALAR_TIER)
            return (f"level up to tier {tier + 1} "
                    f"({_n(obs, SCALAR_UPGRADE_COST)} Gold)")
        if kind == "end":
            return "end turn and start your fight"
        if kind == "hero_power":
            name = self.power_text(_n(obs, SCALAR_POWER_HERO))[0]
            cost = _n(obs, SCALAR_POWER_COST)
            return f"use your hero power {name} ({'free' if cost == 0 else f'{cost} Gold'})"
        if kind == "hero_power2":
            name = self.power_text(_n(obs, SCALAR_POWER2_HERO))[0]
            cost = _n(obs, SCALAR_POWER2_HERO + 1)
            return (f"use your second hero power {name} "
                    f"({'free' if cost == 0 else f'{cost} Gold'})")
        if kind == "choose_trinket":
            opts = self.trinket_offer_options(obs)
            if a < len(opts):
                name, text, price = opts[a]
                return f"take the trinket {name} for {price} Gold" + (
                    f" ({text})" if text else "")
            return f"take trinket offer {a + 1}"
        if kind == "dark_gift":
            return (f"press Dark Discovery ({self.rules.get('dark_gift_cost', 3)} Gold): "
                    "offers three Tavern minions with a Dark Gift each")
        return f"action {index}"

    def pending_card(self, obs):
        """The card a pending choice comes from, "" when the observation has
        no card for it (a hero power or a trinket asking for a target)."""
        cid = _n(obs, SCALAR_PENDING_CARD)
        return self.card_name(cid) if cid else ""

    def _target_suffix(self, obs):
        card = self.pending_card(obs)
        kind = TARGET_KIND_NAMES[0]
        for j in range(TARGET_KINDS):
            if _b(obs, SCALAR_TARGET_KIND + j):
                kind = TARGET_KIND_NAMES[j]
        a, h = _n(obs, SCALAR_PENDING_ATTACK), _n(obs, SCALAR_PENDING_HEALTH)
        stats = f" {a:+d}/{h:+d}" if (a or h) else ""
        who = f" of {card}" if card else " of the effect that asked"
        return f"{who} ({kind}{stats})"

    def describe(self, obs, mask, seat=None):
        """(state text, [(legal action index, words)]) for one decision."""
        return self.state(obs, seat, mask), self.actions(obs, mask)


def main():
    import argparse

    import numpy as np

    a = argparse.ArgumentParser(description="Print one seat's view as text.")
    a.add_argument("--seed", type=int, default=1)
    a.add_argument("--lineup", default="1,0,0,4,3,0",
                   help="learner,snapshot,standard,rush,random,real seats")
    args = a.parse_args()
    lineup = [int(x) for x in args.lineup.split(",")]
    env = bgpy.VecEnv(1, args.seed, lineup)
    obs = np.zeros((1, bgpy.OBS_SIZE), np.float32)
    mask = np.zeros((1, bgpy.N_ACTIONS), bool)
    seat = np.zeros(1, np.int64)
    kind = np.zeros(1, np.int64)
    env.observe(obs, mask, seat, kind)
    d = Descriptor(env)
    state, rows = d.describe(obs[0], mask[0], int(seat[0]))
    print(state)
    print()
    print("LEGAL ACTIONS:")
    for index, words in rows:
        print(f"  {index}: {words}")


if __name__ == "__main__":
    main()
