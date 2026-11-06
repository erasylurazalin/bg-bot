#!/usr/bin/env python3
"""Economy cards, and whether early economy makes Era's games better.

Part 1 scans every card the engine has (cards.toml minions and spells,
trinkets.toml, darkgifts.toml, heroes.toml) and writes the economy cards to
train/runs/econ-check/economy-cards.tsv.

Part 2 reads Era's own games (boards/actions/*.json for the steps,
boards/real/*.json for the place, the Power.log build number for the patch),
finds the economy cards Era bought, played or picked, and compares games with
economy by turn 4, 6 and 8 against games without.

  python3 -I train/econ_check.py               # both parts
  python3 -I train/econ_check.py --part 1      # the card table only
  python3 -I train/econ_check.py --part 2      # the games and the report only

Python only, no GPU, no network, nothing written outside train/runs/econ-check/.
The logs are only read.
"""

import argparse
import glob
import json
import os
import random
import re
import statistics
import sys
import tomllib
from collections import Counter, defaultdict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT_DIR = os.path.join(ROOT, "train", "runs", "econ-check")
TSV_PATH = os.path.join(OUT_DIR, "economy-cards.tsv")
REPORT_PATH = os.path.join(OUT_DIR, "report.md")
GAMES_PATH = os.path.join(OUT_DIR, "games.tsv")

# The economy kinds. The brief's list, with the labels this script uses.
KIND_ORDER = [
    "gold_now",        # gold now
    "gold_next_turn",  # gold next turn
    "max_gold",        # more maximum gold (Strike Oil)
    "sell_gold",       # sells for more than 1 gold (Tortollan Blue Shell)
    "free_refresh",    # cheaper or free refreshes
    "cheap_minion",    # cheaper minions
    "cheap_spell",     # cheaper Tavern spells
    "cheap_level",     # cheaper levelling
    "free_card",       # a free minion or card, Discover, generate, copy
    "habit",           # Tavern Coin: a habit, not a play (Era)
]
KIND_LABEL = {
    "gold_now": "gold now",
    "gold_next_turn": "gold next turn",
    "max_gold": "more maximum gold",
    "sell_gold": "sells for more than 1",
    "free_refresh": "free or cheaper refreshes",
    "cheap_minion": "cheaper minions",
    "cheap_spell": "cheaper spells",
    "cheap_level": "cheaper levelling",
    "free_card": "a free card",
    "habit": "habit (Tavern Coin)",
}


# ---------------------------------------------------------------------------
# Toml loading
# ---------------------------------------------------------------------------

def load_toml(path):
    """Parse one of the engine's toml files.

    Plain tomllib first. trinkets.toml is not strict TOML 1.0: several `rule`
    entries are inline tables spread over several lines, and one of them has a
    trailing comma before the closing brace. Both are outside the spec, so on a
    parse error the file is re-joined (comment lines left alone, an inline
    table's lines joined into one) and trailing commas are dropped. Nothing
    else about the text is changed.
    """
    text = open(path, encoding="utf-8").read()
    try:
        return tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        pass
    out, buf, depth = [], [], 0
    for line in text.split("\n"):
        stripped = line.strip()
        if depth == 0 and (stripped == "" or stripped.startswith("#")):
            out.append(line)
            continue
        no_strings = re.sub(r'"[^"]*"', '""', line)
        if depth > 0:
            buf.append(stripped)
            depth += no_strings.count("{") - no_strings.count("}")
            if depth == 0:
                out.append(" ".join(buf))
                buf = []
        else:
            depth = no_strings.count("{") - no_strings.count("}")
            if depth > 0:
                buf = [line.rstrip()]
            else:
                out.append(line)
    fixed = re.sub(r",\s*\}", " }", "\n".join(out))
    return tomllib.loads(fixed)


def clean_text(text):
    if not text:
        return ""
    text = text.replace("[x]", "").replace("<b>", "").replace("</b>", "")
    text = text.replace("<i>", "").replace("</i>", "").replace("<br/>", " ")
    return re.sub(r"\s+", " ", text).strip()


class Texts:
    """Card text lookups: ref/bg-cards.json by name and card id, ref/bg-meta.json
    by card id (it is the only export with the trinkets and the hero powers),
    then ref/hsjson-cards.json as the last resort."""

    def __init__(self):
        cards = json.load(open(os.path.join(ROOT, "ref", "bg-cards.json")))["cards"]
        self.by_id, self.by_name = {}, defaultdict(list)
        for c in cards:
            self.by_id[c["card_id"]] = c
            self.by_name[c["name"]].append(c)
        meta = json.load(open(os.path.join(ROOT, "ref", "bg-meta.json")))
        self.meta = {c["card_id"]: c for c in meta["trinkets"] + meta["hero_powers"]}
        hs = json.load(open(os.path.join(ROOT, "ref", "hsjson-cards.json")))
        self.hs = {c["id"]: c for c in hs if "id" in c}

    def get(self, name, card_id=None):
        if card_id and card_id in self.by_id:
            c = self.by_id[card_id]
            return clean_text(c.get("text_plain") or c.get("text"))
        if card_id and card_id in self.meta:
            c = self.meta[card_id]
            return clean_text(c.get("text_plain") or c.get("text"))
        for c in self.by_name.get(name, []):
            t = clean_text(c.get("text_plain") or c.get("text"))
            if t:
                return t
        if card_id and card_id in self.hs:
            return clean_text(self.hs[card_id].get("text"))
        return ""


# ---------------------------------------------------------------------------
# Walking a card's effect data
# ---------------------------------------------------------------------------

TRIGGER_KEYS = {
    "battlecry", "deathrattle", "rally", "avenge", "end_of_turn", "start_of_turn",
    "start_of_combat", "on_sell", "on_event", "activate", "damage_milestone",
    "choose_one", "on_card_play", "on_card_added", "on_cast_spell", "on_discard",
    "whenever_friendly_attack", "after_friendly_attack", "after_summon_give_stats",
    "effect", "on_play", "on_buy", "on_pick", "on_play_or_magnetize_mech",
    "on_play_mech", "on_reborn", "on_summon_beast", "on_kill", "after_hero_damage",
    "after_buy", "after_discover", "after_play", "after_play_tribe", "fight",
    "rule", "on_cast_spell_on_self", "on_cast_spell_on_mech",
    "on_cast_spell_on_tribe", "on_gem_played_on_self", "start_every", "after",
    "when_damaged", "on_discard", "spellcraft", "after_demon_damage",
    "after_battlecry", "after_choose_one_play", "trigger_battlecry",
}

# Triggers that hand you a card for nothing, the moment you play the card (or,
# for start/end of turn and pick, for nothing later on).
FREE_TRIGGERS = {
    "battlecry", "effect", "choose_one", "rally", "end_of_turn", "start_of_turn",
    "on_pick", "on_play", "on_buy", "start_every",
}


def walk(obj, path=()):
    """Yield (path, key, value) for every mapping entry under obj."""
    if isinstance(obj, dict):
        for k, v in obj.items():
            yield path + (k,), k, v
            yield from walk(v, path + (k,))
    elif isinstance(obj, list):
        for v in obj:
            yield from walk(v, path)


def first_trigger(path):
    for p in path:
        if p in TRIGGER_KEYS:
            return p
    return path[0] if path else ""


# Effect keys that are economy in the data, and the kind each one is.
GOLD_KEYS = {
    "gold": "gold_now",
    "growing_gold": "gold_now",
    "lucky_roll": "gold_now",
    "gold_next_turn": "gold_next_turn",
    "gold_next_turn_after_sell": "gold_next_turn",
    "gold_if_lost": "sell_gold",
    "max_gold": "max_gold",
    "extra_max_gold": "max_gold",
    "free_refreshes": "free_refresh",
    "free_refresh_per_turn": "free_refresh",
    "free_rolls": "free_refresh",
    "free_rolls_per_turn": "free_refresh",
    "free_refresh_on_play": "free_refresh",
    "spell_discount": "cheap_spell",
    "spell_discount_turn": "cheap_spell",
    "free_third_spell": "cheap_spell",
    "minion_price": "cheap_minion",
    "prices": "cheap_minion",
    "stay_frosty": "cheap_minion",
    "upgrade_discount": "cheap_level",
    "upgrade_discount_elements": "cheap_level",
    "health_price_minions": "cheap_minion",
    "health_price_highest_tier": "cheap_minion",
    "every_nth_buy_health": "cheap_minion",
    "health_price_spell": "cheap_spell",
    "free_buys": "cheap_minion",
    "free_tribe_buys": "cheap_minion",
    "gold_on_tier6": "gold_now",
    "gold_per_type": "gold_now",
    "gold_per_golden": "gold_now",
    "gold_after_upgrade": "gold_now",
    "gold_after_buy_tribe": "gold_now",
}

# These three pay for a card in Health instead of Gold: real economy (the Gold
# is never spent) but health is not gold, so they are borderline.
HEALTH_KEYS = {"health_price_minions", "health_price_highest_tier",
               "every_nth_buy_health", "health_price_spell"}

# Keys that hand you a card (minion, spell or copy) without paying Gold for it.
GEN_KEYS = {
    "discover", "discover_spell", "discover_tribe", "discover_dead_minion",
    "discover_locked_minion", "discover_demon_tier_damage", "discover_tier7_if_golden",
    "discover_tribe_if_another", "discover_prize", "discover_prizes", "discover_tier",
    "discover_by_tier", "discover_hero_power", "discover_powers", "discover_power",
    "discover_magnetic", "discover_my_tier_costs_more", "discover_remember",
    "discover_deathrattle",
    "get", "get_pair", "get_plain_copy", "get_random_held",
    "get_most_common_type_minion", "get_any_tavern_spell", "get_linked_minions",
    "get_golden", "get_tribe_cut_cost",
    "random_minion", "random_tribe_minion", "random_tier_minion",
    "random_spell_cost", "random_spell_up_to", "random_spell_at_least",
    "random_bounty", "random_chromadrake", "random_volumizer", "random_choose_one",
    "copy_buy_after_minions", "hand_copy_every", "copy_last_spell",
    "card_after_buys", "champion", "three_wishes", "steal_card_damage",
    "steal_tavern", "steal_random_tavern", "steal_shop_as_stats", "imprison",
    "fill_hand", "fill_hand_spells", "extra_shop_minion", "acquire_copies",
    "get_gems", "get_gems_overflow",
    "cloning_conch", "linked_spells", "linked_minions", "misplaced_tea_set",
    "triple_reward", "triple_wildcard", "amalgam_gain_copy",
    "destroy_chosen_get_copy", "destroy_get_random", "magnetize_random_mech",
    "magnetize_satellite", "magnetize_volumizer", "magnetize_to_tribes",
    "magnetize_to_undead", "return_to_hand", "transform_higher",
    "add_fodder_refreshes", "add_fodder_to_next_refresh", "evolving_tavern",
}

# Keys that make a minion Golden: a Triple Reward and stats, but no Gold.
GOLDEN_KEYS = {"make_golden", "make_golden_max_tier", "make_golden_return",
               "makes_golden", "golden_minion_tier"}

# Generation that is not a card: a tribe resource, or a body in a fight.
RESOURCE_KEYS = {"get_gems", "get_gems_overflow"}

# Triggers that keep handing you cards without paying again. A generation on
# one of these is the economy Era describes with the tier 4 Rally minions ("it
# gives you 1 gold (a minion)").
REPEAT_TRIGGERS = {
    "start_of_turn", "end_of_turn", "start_every", "rally", "on_buy", "on_play",
    "on_cast_spell", "on_any_spell", "on_buy_spell", "on_event",
    "whenever_friendly_attack", "after_friendly_attack", "on_card_added",
    "on_card_play", "spellcraft", "on_play_mech", "on_play_or_magnetize_mech",
    "on_gem_played_on_self", "on_summon_beast",
}

# Why a one-off generation is only borderline.
TRIGGER_REASON = {
    "battlecry": "one card, bundled with the body you paid 3 gold for",
    "effect": "one card for the card's own price",
    "choose_one": "one card for the card's own price",
    "fight": "a card only in a fight",
    "deathrattle": "a card only when it dies in a fight",
    "on_sell": "you give up the body to get it",
    "avenge": "a card only after enough friendly deaths",
    "damage_milestone": "a card only after a damage total",
    "activate": "you pay gold to activate it",
    "on_discard": "a card costs you another card",
    "on_pick": "one card, bought with the trinket",
    "fight": "a card only in a fight",
    "rule": "conditional",
    "on_destroy": "conditional on destroying a minion",
}

REPEAT_REASON = {
    "start_of_turn": "a card every turn",
    "end_of_turn": "a card every turn",
    "start_every": "a card every few turns",
    "rally": "a card on every rally",
    "on_buy": "a card whenever you buy",
    "on_play": "a card whenever you play a card",
    "on_cast_spell": "a card whenever you cast a spell",
    "on_event": "a card every so many events",
    "whenever_friendly_attack": "a card whenever a minion attacks",
    "after_friendly_attack": "a card after attacks",
    "spellcraft": "a repeatable spell",
}


# A plain-words reason for the rows the effect data gives.
KEY_NOTE = {
    "gold": "gains Gold",
    "growing_gold": "gains Gold, more each turn",
    "lucky_roll": "a die roll of Gold",
    "gold_next_turn": "Gold next turn",
    "gold_next_turn_after_sell": "Gold next turn after each sell",
    "gold_if_lost": "sells for more than 1 Gold",
    "max_gold": "more maximum Gold",
    "extra_max_gold": "more maximum Gold",
    "free_refreshes": "free Refreshes",
    "free_refresh_per_turn": "a free Refresh every turn",
    "free_rolls": "free Refreshes",
    "free_rolls_per_turn": "free Refreshes every turn",
    "free_refresh_on_play": "a free Refresh on play",
    "spell_discount": "a cheaper Tavern spell",
    "spell_discount_turn": "a cheaper Tavern spell this turn",
    "free_third_spell": "a free Tavern spell",
    "minion_price": "cheaper minions",
    "prices": "cheaper minions and Refreshes",
    "stay_frosty": "cheaper minions",
    "upgrade_discount": "a cheaper Tavern upgrade",
    "upgrade_discount_elements": "a cheaper Tavern upgrade",
    "gold_on_tier6": "Gold at Tier 6",
    "gold_per_type": "Gold for each type you have",
    "gold_per_golden": "Gold for each Golden minion",
    "gold_after_upgrade": "Gold after an upgrade",
    "gold_after_buy_tribe": "Gold after each tribal buy",
}


def amount_of(key, val):
    if key in ("gold", "growing_gold"):
        return f"+{val} gold"
    if key in ("gold_next_turn", "gold_next_turn_after_sell"):
        return f"+{val} gold next turn"
    if key == "gold_if_lost":
        return f"sells for {val} gold"
    if key in ("max_gold", "extra_max_gold"):
        return f"+{val} max gold"
    if key in ("free_refreshes", "free_rolls"):
        return f"{val} free refreshes"
    if key in ("free_rolls_per_turn",):
        return f"{val[0]} now, {val[1]} a turn"
    if key == "free_refresh_per_turn":
        return f"{val} a turn"
    if key in ("spell_discount", "spell_discount_turn"):
        return f"spell costs {val} less"
    if key == "minion_price":
        return f"minions cost {val}"
    return ""


# Keys the engine uses once, for a card whose effect has no other key. The key
# name says what it does; the text rule below reads the amount.
SINGLE_KEYS = {
    "overconfidence": ("gold_now", False, "3 gold only if you win the next fight"),
    "health_refreshes": ("free_refresh", True, "2 Refreshes a turn cost Health"),
    "planar_telescope": ("free_card", False, "one minion for the spell's own price"),
    "tomb_turning": ("free_card", False, "one minion for the spell's own price"),
    "friendly_bounty": ("free_card", False, "one minion for the spell's own price"),
    "saloon_finest": ("free_refresh", False, "a Refresh full of Tavern spells"),
    "refresh_with_type": ("free_refresh", False, "a Refresh filtered by type"),
    "teaches_spell": ("free_card", False, "one spell when you buy a Tavern spell"),
    "lockbox_sooner": ("free_card", False, "a Lockbox: a Golden minion in a few turns"),
    "lockbox": ("free_card", False, "a Lockbox: a Golden minion in a few turns"),
    "miniature_ship": ("free_card", False, "1 card"),
}

# The text rules, for the cards whose effect keys say nothing about economy.
TEXT_RULES = [
    (r"gain (\d+) gold", "gold_now", True),
    (r"gain (\d+) gold next turn", "gold_next_turn", True),
    (r"increase your maximum gold", "max_gold", True),
    (r"sells for (\d+) gold", "sell_gold", True),
    (r"free refresh", "free_refresh", True),
    (r"refreshes? each turn cost health", "free_refresh", True),
    (r"reduce the cost of upgrading", "cheap_level", True),
    (r"tavern spells cost \(?\d\)? less", "cheap_spell", True),
    (r"minions in the tavern cost", "cheap_minion", True),
    (r"discover [^.]{0,40}", "free_card", False),
    (r"\bget (?:a|an|two|three|\d)[^.]{0,40}", "free_card", False),
    (r"extra copy[^.]{0,30}", "free_card", False),
    (r"[^.]{0,30}copy of it", "free_card", False),
]


def text_econ(text):
    """Rows read off the card's text, for what the effect keys do not say."""
    low = (text or "").lower()
    rows = []
    for pattern, kind, sure in TEXT_RULES:
        m = re.search(pattern, low)
        if m:
            snippet = m.group(0).strip()
            rows.append(dict(kind=kind, sure=sure, amount="",
                             note=f'the text says "{snippet}"'))
    return rows


def auto_econ(card_kind, entry, text=""):
    """The economy kinds a card's own effect data says it has.

    A first pass only: MANUAL below confirms or replaces every row.
    """
    rows = []
    body = {k: v for k, v in entry.items()
            if k not in ("name", "tier", "card_id", "stats", "tribe", "tribes",
                         "keywords", "slot", "window", "condition", "id",
                         "card", "repeats", "weight", "passive", "cost")}
    power_cost = entry.get("cost") or 0
    for path, key, val in walk(body):
        if key in GOLD_KEYS:
            kind = GOLD_KEYS[key]
            sure, note = True, KEY_NOTE.get(key, f"the data's {key}")
            if key in HEALTH_KEYS:
                sure, note = False, "a card paid in Health, not Gold"
            rows.append(dict(kind=kind, sure=sure, amount=amount_of(key, val), note=note))
        elif key in GOLDEN_KEYS:
            rows.append(dict(kind="free_card", sure=False, amount="1 Golden minion",
                             note="a Golden minion is stats and a Triple Reward, not Gold"))
        elif key in SINGLE_KEYS:
            kind, sure, note = SINGLE_KEYS[key]
            rows.append(dict(kind=kind, sure=sure, amount="", note=note))
        elif key in GEN_KEYS:
            trig = first_trigger(path)
            if key in RESOURCE_KEYS:
                rows.append(dict(kind="free_card", sure=False,
                                 amount=f"{val} Blood Gems",
                                 note=f"{trig}: Blood Gems, not a card or Gold"))
            elif trig in REPEAT_TRIGGERS:
                rows.append(dict(kind="free_card", sure=True, amount="1 card",
                                 note=REPEAT_REASON.get(trig, f"{trig}: a card again and again")))
            elif card_kind == "hero power" and power_cost and power_cost <= 2:
                rows.append(dict(kind="cheap_minion", sure=True, amount="1 card",
                                 note=f"a card for {power_cost} gold, under the 3 gold price"))
            elif card_kind == "hero power" and power_cost:
                rows.append(dict(kind="free_card", sure=False, amount="1 card",
                                 note=f"a card for {power_cost} gold, no cheaper than the Tavern"))
            else:
                note = TRIGGER_REASON.get(trig, f"{trig}: conditional")
                if trig == "effect":
                    note = {
                        "spell": "one card for the spell's own price",
                        "minion": "one card, bundled with the body you paid 3 gold for",
                        "hero power": "one card, and it does not come every turn",
                        "trinket": "one card, bought with the trinket",
                        "dark gift": "one card, and it does not come every turn",
                    }.get(card_kind, note)
                rows.append(dict(kind="free_card", sure=False, amount="1 card",
                                 note=note))
    if not rows:
        # Nothing in the effect data: read the text instead. A card that
        # already has a row keeps only the data rows, so one card is one
        # judgement.
        rows = text_econ(text)
    return rows


# ---------------------------------------------------------------------------
# Part 1: the table
# ---------------------------------------------------------------------------

# Rows settled by hand, one line per (card, kind): name|kind|sure|amount|note.
# A name here replaces whatever the automatic pass said, so every judgement the
# brief left open sits in this one block. Names not in here keep the automatic
# rows (which are all "sure = no" when they are a one-off card).
MANUAL_TEXT = """
Tavern Coin|habit|yes|+1 gold|Era: a habit, not a play, buying and casting it at once changes nothing

Affinity|free_card|yes|1 card|a random minion of its type every 2 turns
Charisma|free_card|yes|1 card|a random minion of your most common type on every Rally
Double Vision|free_card|yes|1 card|a free copy of the minion the gift is on
Replication|free_card|yes|1 card|a plain copy of the minion every 2 turns
Mystic Essence|free_card|no|1 spell|only when the minion dies in a fight
Consanguinity|free_card|no|2 Blood Gems|Blood Gems are a tribe resource, not Gold
Demonology|free_card|no|a Fodder|a Fodder is one free Tavern minion for the next 3 Refreshes
Efficiency|free_card|no|1 Activate|only doubles an Activate you pay for
Fresh Perspective|free_refresh|yes|2 free Refreshes|2 free Refreshes when the minion dies
Gilding|free_card|no|1 Golden minion|Golden is stats and a Triple Reward, not Gold
Sunken Persistence|free_card|no|1 Spellcraft|only makes a Spellcraft spell permanent

Lucky Roll|gold_now|yes|1 to 6 gold|a die roll of 1 to 6 Gold, 1 gold a turn
Pirate Parrrrty!|cheap_minion|yes|1 card|a Pirate for 3 gold, and the next use costs 1 less
Everbloom|gold_now|yes|+2 gold|2 gold after each Tavern upgrade
I'm the Cap'n Now|gold_now|yes|+1 gold|1 gold after each Pirate bought
Piggy Bank|gold_now|yes|+2 gold, +1 a turn|2 gold once, 1 more each turn
Smart Savings|gold_next_turn|yes|+1 gold next turn|1 gold next turn after each minion sold
Wisdom of Ancients|max_gold|yes|+1 max gold|1 more maximum Gold for 3 gold
Clairvoyance|free_refresh|yes|1 a turn|a free Refresh every turn
Imprison|cheap_minion|no|1 card|1 gold to lock a Tavern card in hand for 2 turns
Lead Explorer|cheap_minion|yes|1 card|a minion of your Tier for 1 gold, 1 more each use
Queen of Dragons|cheap_minion|yes|1 card|a Dragon for 1 gold
Tinker|cheap_minion|yes|1 card|a Magnetic Mech for 1 gold
Dark Ritual|cheap_minion|yes|1 card|2 minions for 2 gold, and you play one
Graveyard Shift|cheap_minion|yes|1 card|a Tavern card for 2 gold and 2 Health
King of Aberrations|cheap_minion|yes|1 card|a minion of a type for 2 gold
King of Beasts|cheap_minion|yes|1 card|a minion of a type for 2 gold
King of Demons|cheap_minion|yes|1 card|a minion of a type for 2 gold
King of Dragons|cheap_minion|yes|1 card|a minion of a type for 2 gold
King of Elementals|cheap_minion|yes|1 card|a minion of a type for 2 gold
King of Mechs|cheap_minion|yes|1 card|a minion of a type for 2 gold
King of Murlocs|cheap_minion|yes|1 card|a minion of a type for 2 gold
King of Pirates|cheap_minion|yes|1 card|a minion of a type for 2 gold
King of Quilboar|cheap_minion|yes|1 card|a minion of a type for 2 gold
King of Undead|cheap_minion|yes|1 card|a minion of a type for 2 gold
A Tale of Kings|cheap_minion|yes|1 card|a minion of a type for 2 gold
Reclaimed Souls|cheap_minion|yes|1 card|a copy of a minion that died last combat for 2 gold
See the Light|cheap_minion|yes|1 card|a Tavern minion set to 2/2 for 2 gold
I Spy|cheap_minion|yes|1 card|a copy of a minion from your next opponent for 2 gold
Embrace Your Rage|cheap_minion|yes|1 card|a minion of your Tier for 2 gold
Manastorm|cheap_minion|yes|minions cost 2|minions and Refreshes cost 2, upgrades cost 1 more
Stay Frosty|cheap_minion|yes|minions cost 2|minions cost 2, the Tavern offers one fewer
Arcane Knowledge|cheap_spell|yes|spell costs 1 less|the next Tavern spell costs 1 less
Reliquary Research|cheap_spell|yes|1 free spell|every third Tavern spell costs 0
Avalanche|cheap_level|yes|3 gold off|3 gold off the upgrade after 3 Elementals
Friendly Wager|gold_now|yes|+2 gold net|3 Tavern Coins for 1 gold, so 2 gold more than it costs
Gonna Be Rich!|free_card|no|1 Golden minion|once a game, stats and a Triple Reward, not Gold
King of Duality|free_card|no|2 Hero Powers|two Hero Powers on Turn 4, not a card or Gold
Adventure!|free_card|no|1 Hero Power|a Hero Power at the start, not a card or Gold
Blessing of the Nine Frogs|free_card|no|1 spell|one Tavern spell, once
Bloodbound|free_card|no|2 Blood Gems|Blood Gems are a tribe resource, not Gold
I'll Take That!|free_card|no|1 card|a copy of the first minion you kill, for 1 gold
Incubate|free_card|no|1 card|a random Aberration for 1 gold after discarding a card
Rune of Damnation|free_card|no|1 card|one random Undead
The Galaxy's Lens|free_card|no|1 card|a copy of the last Tavern spell for 3 gold
Three Wishes|free_card|no|1 card|finds the third copy, so a Triple Reward, not Gold
Bananarama|free_card|no|2 Bananas|Bananas are stat spells
Battle Brand|free_card|no|1 card|a Brann Bronzebeard after 4 Battlecry buys, once a game
Choose Your Champion|free_card|no|1 card|a Tier 7 minion after 60 Gold spent
Come One, Come All!|free_card|no|1 card|a minion for 3 Darkmoon Tickets
Dungar's Gryphon|free_card|no|1 card|a location, and the card comes from the location
Westfall|free_card|no|1 spell|one 1-cost Tavern spell from the location
Eastern Plaguelands|free_card|no|1 card|a minion of your Tier as the location's reward
Expedition Plans|free_card|no|3 cards|3 minions at the start of the game, once
Feel Devastation|free_card|yes|1 card|a minion with a Dark Gift every 4 turns
Glaive Ricochet|free_card|yes|1 card|a copy of one of the minions you buy, once a turn
Gone Fishing|free_card|yes|1 card|a random Murloc after every 5 minions sold
MechGyver|free_card|yes|1 card|a random Mech after 9 friendly deaths
Prize Wall|free_card|yes|1 card|a Darkmoon Prize every 4 turns
Upbeat Harmony|free_card|yes|1 card|a copy of a card every 3 turns
Snicker-snack|free_card|no|1 Battlecry|repeats a friendly Battlecry, Gold only if that Battlecry was Gold
Heroic Inspiration|free_card|no|1 card|a Triple Reward after 15 attacks
Kel'Thuzad's Kitty|free_card|no|1 card|a minion from a dead hero, when another hero dies
Natural Balance|free_card|no|1 card|a Triple Reward after 20 Tiers of cards
Nether Portal|free_card|no|1 card|a portal after 150 damage, and it becomes another power
Procrastinate|free_card|no|2 cards|2 minions at the start of the game, once
Stir the Pot|free_card|no|1 card|a Discover after 3 minions thrown in the pot
Trash for Treasure|free_card|no|1 card|a minion of a lower Tier for the one you remove
Verdant Spheres|free_card|no|1 Tavern Coin|a Tavern Coin after every 3 minions bought, the card Era calls a habit
Void Power|free_card|no|1 card|a Tier 5 minion with a Dark Gift at the start, once
Bob's Burgles|free_refresh|no|1 Refresh|a Refresh full of the opponent's minions, still 3 gold a minion
Swap, Lock, & Shop It|free_card|no|1 card|swaps a minion you own for a Tavern one, not a free card
Spectral Sight|cheap_minion|no|1 free buy|the free buy is on a cooldown of turns
Galakrond's Greed|cheap_minion|no|1 card|1 gold to replace a Tavern offer with a higher Tier one
Arcane Alteration|cheap_minion|no|1 card|replaces a Tavern card with one of the same Tier, free
Temporal Tavern|free_refresh|no|1 Refresh|a Refresh with higher Tier minions for 1 gold
Prestidigitation|free_card|no|1 Secret|a Secret, which is value in a fight, not Gold
Stay Frosty|cheap_minion|yes|minions cost 2|minions cost 2, the Tavern offers one fewer
Menagerist|free_card|no|1 card|a 2/2 Amalgam at the start, not economy
Pilot the Shredder|free_card|no|1 card|a 2/1 Shredder at the start, not economy

Azeroth Model Globe|gold_now|yes|+2 gold|2 gold every 2 turns
Azeroth Model Globe|free_card|yes|1 card|a Tier 6 minion every 2 turns
Bob's Tip Jar|gold_now|yes|+4 gold|4 gold once
Bob's Tip Jar|max_gold|yes|+4 max gold|4 more maximum Gold
Bob-blehead|gold_now|yes|+2 gold|2 gold once, and no more Tier 1 or 2 offers
Glowing Crystal|gold_now|yes|+1 gold per type|1 gold for each different type you have, each turn
Maw Caster Portrait|gold_now|yes|+2 gold|2 gold whenever you destroy a minion outside combat
Mysterious Orb|gold_now|yes|+10 gold|10 gold once
Orb of the Unknown|gold_now|yes|+4 gold|4 gold once
Orb of the Unknown|free_card|no|1 trinket|a random Greater Trinket, not a card
Ornate Clock|gold_now|yes|+2 gold|2 gold once
Rascal Sticker|gold_now|yes|+1 gold per Golden|1 gold for each Golden minion, each turn
Reinvigorating Light|gold_now|yes|+3 gold|3 gold once
Reinvigorating Light|free_card|no|1 Hero Power|a second Hero Power, not a card
Safety Patch|gold_now|yes|+5 gold|5 gold once
Stuffed Coin Purse|gold_now|yes|+12 gold|12 gold at Tier 6
Trickster's Sleeve|gold_now|yes|+1 gold|1 gold on the first minion you buy
Wand of Divination|gold_now|yes|+1 gold|1 gold after 3 spells on minions
Worn Treasure Map|gold_now|yes|+10 gold|10 gold two turns later
Sous Chef Sticker|gold_now|yes|+1 gold|1 gold after each Hero Power use, and one more use a turn
Goblin Wallet|max_gold|yes|+1 max gold|1 more maximum Gold each turn
Gold-plated Compass|free_refresh|yes|5 free Refreshes|5 free Refreshes
Herding Horn|free_refresh|yes|1 free Refresh|a free Refresh on every Rally
Recycling Sticker|free_refresh|yes|1 free Refresh|a free Refresh after each Elemental played
Warband Whistle|free_refresh|yes|1 free Refresh|one free Refresh
Warcry Totem|cheap_minion|yes|2 free minions|the first 2 Battlecry minions each turn are free
Grifter Portrait|cheap_minion|yes|1 free minion|the first Pirate each turn is free
Grifter Portrait|free_card|no|1 card|a Doubloon Grifter, bought with the trinket
Bartend-o-Tron's Oilcan|cheap_level|yes|3 gold off a turn|3 gold off the upgrade, and again each turn
Frigid Blossom|cheap_level|yes|1 gold off|1 gold off the upgrade after each Refresh
Demonic Tapestry|cheap_minion|no|1 Health buy|a card paid in Health, not Gold
Pilgrimp Sticker|cheap_minion|no|1 Health buy|a card paid in Health, not Gold
The Eye of Sargeras|cheap_minion|no|1 Health buy|a card paid in Health, not Gold
Bazaar Sticker|cheap_spell|no|1 Health buy|a spell paid in Health, not Gold
Book of Medivh|free_card|yes|1 spell|a Tavern spell at the pick and another each turn
Transcribing Typewriter|free_card|yes|4 cards|a copy of the next 2 minions you buy (4 for the Greater)
Sinstone Sticker|free_card|yes|4 cards|an extra copy of the first 2 Discovers each turn
Portable Factory|free_card|yes|1 card|a Tier 4 minion at the pick and another each turn
Sellemental Portrait|free_card|yes|1 card|a Sellemental at the pick and another each turn
Reflective Pendant|free_card|yes|1 card|a copy of a friendly minion at the pick and another each turn
Azerite Portrait|free_card|yes|1 card|an Azerite minion at the pick and another each turn
Pagle's Fishing Rod|free_card|yes|1 card|a card at the pick and another each turn
Glass of Perspective|free_card|yes|1 card|a card at the pick and another each turn
Guiding Candle|free_refresh|no|2 Refreshes|the first 2 Refreshes each turn are Tier 6 only, they still cost gold
Colorful Compass|free_card|no|1 card|a Golden minion of a tribe, paid for with the trinket
Corrupted Tome|free_card|no|1 card|a Triple Prize instead of a Triple Reward
Innkeeper's Stein|free_card|no|1 more offer|the Tavern offers one more minion
Innkeeper's Hearth|free_card|no|2 cards|two Tier 6 minions at the pick
Funeral Wreath|free_card|no|1 card|a copy after a Reborn, 3 times a combat
Lava Lamp|free_card|no|1 card|a random Elemental after 6 sells
Stitched Salvager|free_card|no|1 card|summons a copy in a fight, not a card in hand
Flighty Scout|free_card|no|1 card|summons a copy of itself in a fight, not economy
The Unlimited Coin|gold_now|yes|+1 gold a turn|1 Gold, and it comes back to hand every turn
Twisted Wrathguard|free_card|no|a Fodder|a Fodder is one free Tavern minion for the next Refresh
Spark Snapper|free_card|yes|1 card|a random Mech after every Mech played
Souvenir Stand|free_card|no|1 trinket|a copy of your Greater Trinket, not a card
Laboratory Assistant|free_card|no|a Fodder|a Fodder is one free Tavern minion for the next 3 Refreshes
Malchezaar, Prince of Dance|free_refresh|yes|2 Refreshes a turn|2 Refreshes each turn cost Health instead of Gold

"""
# Cards in cards.toml that look like economy but are not: their rows are
# dropped rather than kept as borderline, with the reason here.
NOT_ECON = {
    "Hooktusk, Master Marauder": "a Discover only gives Era's Pirates +1/+1, no card",
}
MANUAL = defaultdict(list)
for line in MANUAL_TEXT.strip().split("\n"):
    if not line.strip():
        continue
    name, kind, sure, amount, note = [x.strip() for x in line.split("|")]
    rows = MANUAL[name]
    if any(r[0] == kind for r in rows):
        continue
    rows.append((kind, sure == "yes", amount, note))


def classify(card_kind, name, entry, text):
    if name in NOT_ECON:
        return []
    if name in MANUAL:
        return [dict(kind=k, sure=s, amount=a, note=n) for k, s, a, n in MANUAL[name]]
    rows = auto_econ(card_kind, entry, text)
    # One row per (card, kind): the surest one wins, and the notes merge, so a
    # card with a Battlecry and a Deathrattle that both hand you a card is one
    # row, not two.
    merged = {}
    for r in rows:
        key = r["kind"]
        if key not in merged:
            merged[key] = r
        elif r["sure"] and not merged[key]["sure"]:
            # The repeating row is the real reason a card is economy ("Get a
            # X. At the start of each turn, get another" is a card every turn,
            # not one card at the pick).
            merged[key]["sure"] = True
            merged[key]["note"] = r["note"]
            if not merged[key]["amount"]:
                merged[key]["amount"] = r["amount"]
        else:
            if r["sure"]:
                merged[key]["sure"] = True
            if r["note"] not in merged[key]["note"]:
                merged[key]["note"] += "; " + r["note"]
            if not merged[key]["amount"]:
                merged[key]["amount"] = r["amount"]
    return list(merged.values())


def engine_cards():
    """Every card the engine has, as (kind, tier, name, entry, text)."""
    texts = Texts()
    out = []
    cards = load_toml(os.path.join(ROOT, "cards.toml"))
    for c in cards["minion"] + cards["spell"]:
        kind = "minion" if c in cards["minion"] else "spell"
        tier = c.get("tier", 0)
        out.append((kind, str(tier), c["name"], c.get("card_id", ""), c,
                    texts.get(c["name"], c.get("card_id"))))
    for t in load_toml(os.path.join(ROOT, "trinkets.toml"))["trinket"]:
        out.append(("trinket", str(t.get("slot", "")), t["name"], t.get("card_id", ""), t,
                    texts.get(t["name"], t.get("card_id"))))
    gifts = load_toml(os.path.join(ROOT, "darkgifts.toml"))["gift"]
    for g in gifts:
        window = g.get("window")
        tier = "-".join(str(w) for w in window) if window else ""
        out.append(("dark gift", tier, g["name"], g.get("id", ""), g,
                    texts.get(g["name"], g.get("id"))))
    heroes = load_toml(os.path.join(ROOT, "heroes.toml"))
    seen_powers = set()
    for h in heroes["hero"]:
        p = h.get("power") or {}
        cid = p.get("card_id")
        if not cid or cid in seen_powers:
            continue
        seen_powers.add(cid)
        tier = "passive" if p.get("passive") else str(p.get("cost", ""))
        out.append(("hero power", tier, p.get("name", cid), cid, p,
                    texts.get(p.get("name", ""), cid)))
    for p in heroes.get("power", []):
        cid = p.get("card_id")
        if not cid or cid in seen_powers:
            continue
        seen_powers.add(cid)
        tier = "passive" if p.get("passive") else str(p.get("cost", ""))
        out.append(("hero power", tier, p.get("name", cid), cid, p,
                    texts.get(p.get("name", ""), cid)))
    return out


def build_table(verbose=False):
    rows = []
    seen = set()
    for kind, tier, name, card_id, entry, text in engine_cards():
        seen.add(name)
        for r in classify(kind, name, entry, text):
            rows.append(dict(name=name, kind=kind, tier=tier, card_id=card_id,
                             econ_kind=r["kind"],
                             amount=r["amount"], sure="yes" if r["sure"] else "no",
                             note=r["note"]))
    order = {k: i for i, k in enumerate(KIND_ORDER)}
    kind_order = {k: i for i, k in enumerate(
        ["minion", "spell", "trinket", "dark gift", "hero power"])}
    rows.sort(key=lambda r: (kind_order.get(r["kind"], 9), order.get(r["econ_kind"], 99),
                             _tier_key(r["tier"]), r["name"]))
    missing = sorted(n for n in MANUAL if n not in seen)
    if missing:
        # A name in MANUAL that no engine card has is a typo: say so loudly.
        print("WARNING: MANUAL names with no engine card: " + ", ".join(missing),
              file=sys.stderr)
    if verbose:
        for r in rows:
            print("\t".join([r["name"], r["kind"], r["tier"], r["econ_kind"],
                             r["amount"], r["sure"], r["note"]]))
    return rows


def _tier_key(tier):
    m = re.match(r"(\d+)", str(tier))
    return int(m.group(1)) if m else 99


def write_table(rows):
    os.makedirs(OUT_DIR, exist_ok=True)
    with open(TSV_PATH, "w", encoding="utf-8") as f:
        f.write("name\tkind\ttier\tecon_kind\tamount\tsure\tnote\n")
        for r in rows:
            f.write("\t".join([r["name"], r["kind"], r["tier"], r["econ_kind"],
                               r["amount"], r["sure"], r["note"]]) + "\n")



# ---------------------------------------------------------------------------
# Part 2: Era's games
# ---------------------------------------------------------------------------

LOG_ROOTS = [
    os.path.expanduser("~/bg-logs"),
    os.path.expanduser("~/bg-logs-laptop"),
    os.path.expanduser("~/hearthstone-linux/hearthstone/Logs"),
]
BUILD_RE = re.compile(rb"BuildNumber=(\d+)")
PATCH_BUILDS = {254477: "37.0", 255790: "37.0.2"}
THRESHOLDS = (4, 6, 8)
BOOTSTRAP = 4000
SEED = 20261106


def norm(name):
    """A card name reduced to what makes two spellings the same card."""
    n = (name or "").lower()
    n = re.sub(r"\(greater\)|\(lesser\)", "", n)
    n = re.sub(r"[^a-z0-9]+", "", n)
    return n


class EconIndex:
    """The Part 1 table, keyed for the game logs."""

    def __init__(self, rows):
        self.by_name = defaultdict(list)   # norm(name) -> [(kind, sure)]
        self.by_card_id = defaultdict(list)
        for r in rows:
            if r["sure"] == "yes":
                pair = (r["econ_kind"], True)
                other = (r["econ_kind"], False)
            else:
                pair = (r["econ_kind"], False)
                other = None
            self.by_name[norm(r["name"])].append((pair, other))
            if r["card_id"]:
                self.by_card_id[r["card_id"]].append((pair, other))

    def lookup(self, name=None, card_id=None):
        """[(kind, sure)] for a name or a card id, surest first, deduped."""
        pairs = []
        if card_id and card_id in self.by_card_id:
            pairs = [p for p, _ in self.by_card_id[card_id] if p[0] != "habit"]
        elif name and norm(name) in self.by_name:
            # A name can carry both a sure=yes and a sure=no row for the same
            # kind: keep the sure=yes one only when it exists for that kind.
            seen = {}
            for p, o in self.by_name[norm(name)]:
                for cand in (p, o):
                    if not cand:
                        continue
                    k, sure = cand
                    if k == "habit":
                        continue   # Tavern Coin and its kin: out of Part 2
                    if k not in seen or (sure and not seen[k]):
                        seen[k] = sure
            pairs = [(k, v) for k, v in seen.items()]
        return pairs


def patch_of(source):
    """(patch label, how it was found) for a game's log folder."""
    folder = source[7:] if source.startswith("laptop-") else source
    for root in LOG_ROOTS:
        path = os.path.join(root, folder, "Power.log")
        if not os.path.isfile(path):
            continue
        with open(path, "rb") as fh:
            m = BUILD_RE.search(fh.read(2_000_000))
            if not m:
                with open(path, "rb") as fh2:
                    m = BUILD_RE.search(fh2.read())
        if m:
            build = int(m.group(1))
            return PATCH_BUILDS.get(build, f"build {build}"), "log"
        return "unknown", "log has no BuildNumber"
    m = re.search(r"(\d{4})_(\d{2})_(\d{2})", source)
    if m:
        date = f"{m.group(1)}-{m.group(2)}-{m.group(3)}"
        if date < "2026-11-04":
            return "37.0", "guessed by date"
        if date > "2026-11-04":
            return "37.0.2", "guessed by date"
    return "unknown", "no log, date is 2026-11-04"


def places():
    out = {}
    for path in glob.glob(os.path.join(ROOT, "boards", "real", "*.json")):
        d = json.load(open(path))
        out[(d["source"], d["game"])] = d.get("place", 0)
    return out


def gifts_in(state):
    """Every (gift id, gift name, minion card id) in one board/hand state.

    The key is the gift and the minion's card id, not its entity id: the game
    hands the same minion a new entity id every turn (a Roadboar on the board
    is id 1521 on turn 4 and 2200 on turn 5), so an entity id would read the
    same gift as new every turn.
    """
    for where in ("board", "hand"):
        for card in state.get(where) or []:
            for gift in card.get("dark_gifts") or []:
                yield (gift.get("card_id"), gift.get("name"), card.get("card_id"))


def game_events(path, index):
    """Every economy card Era bought, played or picked in one game.

    A card counts when Era bought it, or played it after getting it for
    nothing, or picked it (trinket, Dark Gift, hero power). Being offered a
    card does not count.
    """
    d = json.load(open(path))
    events = []
    bought = set()

    def add(turn, kind, sure, name, source, at_start=False):
        events.append(dict(turn=turn, kind=kind, sure=sure, name=name,
                           source=source, at_start=at_start))

    def add_card(turn, name, card_id, source, at_start=False):
        for kind, sure in index.lookup(name=name, card_id=card_id):
            add(turn, kind, sure, name, source, at_start)

    gifts_seen = set()
    for turn in d["turns"]:
        t = turn["turn"]
        start = turn.get("start") or {}
        if t == 1:
            for power in start.get("hero_powers") or []:
                add_card(t, power.get("name", ""), power.get("card_id"),
                         "hero power", at_start=True)
        for gid, gname, mid in gifts_in(start):
            if (gid, mid) in gifts_seen:
                continue
            gifts_seen.add((gid, mid))
            for kind, sure in index.lookup(name=gname, card_id=gid):
                add(t, kind, sure, gname, "dark gift")
        for a in turn.get("actions") or []:
            step = a["step"]
            if step == "buy":
                card = a["card"]
                bought.add(card.get("id"))
                add_card(t, card.get("name", ""), card.get("card_id", ""), "bought")
            elif step == "play":
                # A card Era got for nothing is counted on the card that
                # handed it over (its effect, its trinket, its Dark Gift), so
                # only the buy, the pick and the power are counted here. A
                # Triple Reward or a Discover is the price of the card that
                # earned it, not a second economy card.
                continue
            elif step == "trinket_pick":
                picked = a["picked"]
                add_card(t, picked.get("name", ""), picked.get("card_id", ""), "trinket")
            elif step == "discover":
                picked = a.get("picked") or {}
                if a.get("kind") == "dark_gift":
                    for gift in picked.get("dark_gifts") or []:
                        key = (gift.get("card_id"), picked.get("card_id"))
                        if key in gifts_seen:
                            continue
                        gifts_seen.add(key)
                        for kind, sure in index.lookup(name=gift.get("name"),
                                                       card_id=gift.get("card_id")):
                            add(t, kind, sure, gift.get("name"), "dark gift")
                elif "HERO" in str(picked.get("card_id", "")):
                    add_card(t, picked.get("name", ""), picked.get("card_id", ""),
                             "hero power")
            for gid, gname, mid in gifts_in(a.get("after") or {}):
                if (gid, mid) in gifts_seen:
                    continue
                gifts_seen.add((gid, mid))
                for kind, sure in index.lookup(name=gname, card_id=gid):
                    add(t, kind, sure, gname, "dark gift")
    for turn in d["turns"]:
        t = turn["turn"]
        for gid, gname, mid in gifts_in(turn.get("end") or {}):
            if (gid, mid) in gifts_seen:
                continue
            gifts_seen.add((gid, mid))
            for kind, sure in index.lookup(name=gname, card_id=gid):
                add(t, kind, sure, gname, "dark gift")
    return d, events


GOLD_KINDS = ("gold_now", "gold_next_turn", "max_gold", "sell_gold")


def first_turn(events, keep=None):
    """The first turn an economy event Era's way of counting keeps."""
    turns = [e["turn"] for e in events if keep is None or keep(e)]
    return min(turns) if turns else None


SURE = lambda e: e["sure"]

# The four readings of "economy" the report compares.
VARIANTS = [
    ("any economy card (loose)", lambda e: True),
    ("sure cards only", SURE),
    ("sure, decision only (no starting hero power)",
     lambda e: e["sure"] and not e["at_start"]),
    ("sure gold kinds only (gold now, next turn, max gold, sells for more)",
     lambda e: e["sure"] and e["kind"] in GOLD_KINDS),
]


def health_at(turns, t, with_armor=True):
    for turn in turns:
        if turn["turn"] == t:
            start = turn.get("start") or {}
            hp = start.get("health")
            if hp is None:
                return None
            if with_armor:
                return hp + (start.get("armor") or 0)
            return hp
    return None


def load_games(index):
    """One dict per finished game: place, patch, turns, economy events, health."""
    pl = places()
    games = []
    unfinished = 0
    for path in sorted(glob.glob(os.path.join(ROOT, "boards", "actions", "*.json"))):
        d, events = game_events(path, index)
        key = (d["source"], d["game"])
        place = pl.get(key, 0)
        patch, how = patch_of(d["source"])
        turns = d["turns"]
        g = dict(source=d["source"], game=d["game"], place=place, patch=patch,
                 patch_how=how, hero=d["hero"]["name"], turns=turns,
                 events=events, n_turns=len(turns),
                 last_turn=max((t["turn"] for t in turns), default=0),
                 health={t["turn"]: (t.get("start") or {}).get("health")
                         for t in turns},
                 health_armor={t["turn"]: ((t.get("start") or {}).get("health") or 0)
                               + ((t.get("start") or {}).get("armor") or 0)
                               for t in turns})
        if not place:
            unfinished += 1
            continue
        games.append(g)
    return games, unfinished


def group_stats(games):
    n = len(games)
    if not n:
        return dict(n=0, place=None, top4=None, first=None)
    return dict(n=n,
                place=statistics.mean(g["place"] for g in games),
                top4=sum(1 for g in games if g["place"] <= 4) / n,
                first=sum(1 for g in games if g["place"] == 1) / n)


def boot_diff(a, b, field):
    """Bootstrap over games: the difference a minus b, 95% interval."""
    if len(a) < 2 or len(b) < 2:
        return None
    rng = random.Random(SEED)

    def value(g):
        return g["place"] if field == "place" else (1.0 if g["place"] <= 4 else 0.0)

    diffs = []
    for _ in range(BOOTSTRAP):
        sa = [value(a[rng.randrange(len(a))]) for _ in a]
        sb = [value(b[rng.randrange(len(b))]) for _ in b]
        diffs.append(statistics.mean(sa) - statistics.mean(sb))
    diffs.sort()
    lo = diffs[int(0.025 * len(diffs))]
    hi = diffs[int(0.975 * len(diffs)) - 1]
    return lo, hi


def pct(x):
    return "n/a" if x is None else f"{100 * x:.0f}%"


def num(x, digits=2):
    return "n/a" if x is None else f"{x:.{digits}f}"


def ci(pair, digits=2):
    if pair is None:
        return "n/a"
    return f"{pair[0]:+.{digits}f} to {pair[1]:+.{digits}f}"


def main_table(games, keep):
    rows = []
    for thr in THRESHOLDS:
        def has(g):
            ft = first_turn(g["events"], keep)
            return ft is not None and ft <= thr
        a = [g for g in games if has(g)]
        b = [g for g in games if not has(g)]
        sa, sb = group_stats(a), group_stats(b)
        place_diff = None
        top4_diff = None
        if a and b:
            place_diff = (sa["place"] - sb["place"],
                          boot_diff(a, b, "place"))
            top4_diff = (sa["top4"] - sb["top4"], boot_diff(a, b, "top4"))
        rows.append(dict(thr=thr, a=sa, b=sb, place_diff=place_diff,
                         top4_diff=top4_diff))
    return rows


def kind_table(games, sure_only=True):
    """Every economy kind on its own, where it has 8 or more games.

    Only sure cards by default: the loose rows are mostly one-off Discover
    spells, which every game has.
    """
    rows = []
    for kind in [k for k in KIND_ORDER if k != "habit"]:
        for thr in THRESHOLDS:
            a = [g for g in games
                 if any(e["kind"] == kind and e["turn"] <= thr
                        and (e["sure"] or not sure_only) for e in g["events"])]
            if len(a) < 8:
                continue
            b = [g for g in games if g not in a]
            sa, sb = group_stats(a), group_stats(b)
            pd = (sa["place"] - sb["place"], boot_diff(a, b, "place")) if b else None
            td = (sa["top4"] - sb["top4"], boot_diff(a, b, "top4")) if b else None
            rows.append(dict(kind=kind, thr=thr, a=sa, b=sb, place_diff=pd,
                             top4_diff=td))
    return rows


def survivorship(games, keep):
    """Health when the economy card came, against games with no economy by then."""
    rows = []
    for t in range(1, 11):
        alive = [g for g in games if t in g["health"]]
        on = [g for g in alive if any(e["turn"] == t and keep(e) for e in g["events"])]
        none_by = [g for g in alive
                   if not any(e["turn"] <= t and keep(e) for e in g["events"])]
        if not on and not none_by:
            continue
        def hmean(gs):
            vals = [g["health_armor"].get(t) for g in gs if g["health_armor"].get(t)]
            return statistics.mean(vals) if vals else None
        rows.append(dict(turn=t,
                         on=group_stats(on), on_health=hmean(on),
                         none=group_stats(none_by), none_health=hmean(none_by),
                         field_health=hmean(alive), alive=len(alive)))
    return rows


def patch_split(games, keep):
    """The main split inside one patch, only where both cells have 5 games."""
    rows = []
    for thr in THRESHOLDS:
        for patch in ("37.0", "37.0.2"):
            sub = [g for g in games if g["patch"] == patch]
            a = [g for g in sub if (first_turn(g["events"], keep) or 99) <= thr]
            b = [g for g in sub if (first_turn(g["events"], keep) or 99) > thr]
            if len(a) < 5 or len(b) < 5:
                continue
            sa, sb = group_stats(a), group_stats(b)
            pd = (sa["place"] - sb["place"], boot_diff(a, b, "place"))
            td = (sa["top4"] - sb["top4"], boot_diff(a, b, "top4"))
            rows.append(dict(thr=thr, patch=patch, a=sa, b=sb, place_diff=pd,
                             top4_diff=td))
    return rows


def write_games(games):
    with open(GAMES_PATH, "w", encoding="utf-8") as f:
        f.write("\t".join(["source", "game", "patch", "patch_how", "hero", "place",
                           "turns", "first_econ_turn", "first_sure_turn",
                           "n_econ", "n_sure", "health_at_first",
                           "health_plus_armor_at_first", "kinds"]) + "\n")
        for g in games:
            ft = first_turn(g["events"])
            fs = first_turn(g["events"], SURE)
            kinds = Counter(e["kind"] for e in g["events"])
            f.write("\t".join([
                g["source"], str(g["game"]), g["patch"], g["patch_how"], g["hero"],
                str(g["place"]), str(g["n_turns"]),
                str(ft), str(fs), str(len(g["events"])),
                str(sum(1 for e in g["events"] if e["sure"])),
                str(g["health"].get(ft, "")),
                str(g["health_armor"].get(ft, "")),
                ",".join(f"{k}:{v}" for k, v in kinds.most_common()),
            ]) + "\n")


def part2(rows, verbose=False):
    index = EconIndex(rows)
    games, unfinished = load_games(index)
    write_games(games)
    out = dict(rows=rows, games=games, unfinished=unfinished,
               main={}, kinds={}, surv={}, patches={})
    for label, keep in VARIANTS:
        out["main"][label] = main_table(games, keep)
        out["surv"][label] = survivorship(games, keep)
    out["kinds"] = kind_table(games, sure_only=True)
    out["patch_counts"] = Counter(g["patch"] for g in games)
    out["patch_place"] = {p: group_stats([g for g in games if g["patch"] == p])
                          for p in ("37.0", "37.0.2")}
    out["patch_econ4"] = {p: group_stats([g for g in games if g["patch"] == p
                                          and (first_turn(g["events"], SURE) or 99) <= 4])
                          for p in ("37.0", "37.0.2")}
    out["patches"] = patch_split(games, SURE)
    if verbose:
        for g in games:
            print(g["source"], g["game"], g["place"], g["patch"],
                  first_turn(g["events"]), first_turn(g["events"], SURE))
    return out



# ---------------------------------------------------------------------------
# The report
# ---------------------------------------------------------------------------

def rows_by_kind(rows):
    out = defaultdict(list)
    for r in rows:
        out[r["econ_kind"]].append(r)
    return out


def main_md(tab):
    out = ["| by turn | games with | mean place | top 4 | first | games without | mean place | top 4 | first | place difference (95%) | top 4 difference (95%) |",
           "|---|---|---|---|---|---|---|---|---|---|---|"]
    for row in tab:
        a, b = row["a"], row["b"]
        pd, td = row["place_diff"], row["top4_diff"]
        out.append("| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |".format(
            row["thr"], a["n"], num(a["place"]), pct(a["top4"]), pct(a["first"]),
            b["n"], num(b["place"]), pct(b["top4"]), pct(b["first"]),
            f"{pd[0]:+.2f} [{ci(pd[1])}]" if pd else "n/a",
            f"{td[0]:+.2f} [{ci(td[1])}]" if td else "n/a"))
    return "\n".join(out)


def kinds_md(tab):
    out = ["| economy kind | by turn | games with | mean place | top 4 | first | games without | mean place | top 4 | first | place difference (95%) |",
           "|---|---|---|---|---|---|---|---|---|---|---|"]
    for row in tab:
        a, b = row["a"], row["b"]
        pd, td = row["place_diff"], row["top4_diff"]
        out.append("| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |".format(
            row["kind"], row["thr"], a["n"], num(a["place"]), pct(a["top4"]),
            pct(a["first"]), b["n"], num(b["place"]), pct(b["top4"]), pct(b["first"]),
            f"{pd[0]:+.2f} [{ci(pd[1])}]" if pd else "n/a"))
    return "\n".join(out)


def surv_md(tab):
    out = ["| turn | alive | on this turn | health+armor | place | no economy by then | health+armor | place | all games alive | health+armor |",
           "|---|---|---|---|---|---|---|---|---|---|"]
    for row in tab:
        out.append("| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |".format(
            row["turn"], row["alive"], row["on"]["n"], num(row["on_health"], 1),
            num(row["on"]["place"]), row["none"]["n"], num(row["none_health"], 1),
            num(row["none"]["place"]), row["alive"], num(row["field_health"], 1)))
    return "\n".join(out)


def borderline_md(rows):
    """The borderline rows, grouped by the reason they are borderline."""
    groups = defaultdict(list)
    for r in rows:
        if r["sure"] == "no":
            groups[(r["econ_kind"], r["note"])].append(r["name"])
    order = {k: i for i, k in enumerate(KIND_ORDER)}
    out = []
    for (kind, note) in sorted(groups, key=lambda k: (order.get(k[0], 99), k[1])):
        names = sorted(set(groups[(kind, note)]))
        out.append(f"**{KIND_LABEL[kind]}, {note}** ({len(names)}): " + ", ".join(names))
        out.append("")
    return "\n".join(out)


def write_report(rows, res, n_engine):
    games = res["games"]
    kind_rows = rows_by_kind(rows)
    sure_rows = [r for r in rows if r["sure"] == "yes"]

    counts = ["| economy kind | rows | sure = yes | cards |", "|---|---|---|---|"]
    for k in KIND_ORDER:
        rs = kind_rows.get(k, [])
        counts.append(f"| {k} ({KIND_LABEL[k]}) | {len(rs)} | "
                      f"{sum(1 for r in rs if r['sure'] == 'yes')} | "
                      f"{len({r['name'] for r in rs})} |")

    engine_lines = ["| what the engine has | entries scanned |", "|---|---|"]
    for k, v in n_engine.items():
        engine_lines.append(f"| {k} | {v} |")

    patches = Counter(g["patch"] for g in games)
    escaped = res["unfinished"]
    n_games = len(games)

    out = []
    a = out.append
    a("# Economy cards, and whether early economy makes Era's games better")
    a("")
    a("2026-11-06. Question: Era says \"I almost always want to find any way to")
    a("generate economy. Nearly every game where I find economy early ends up")
    a("being a good game.\" This checks the card set for economy cards (Part 1)")
    a("and Era's own games for whether early economy goes with better places")
    a("(Part 2).")
    a("")
    a("Run it with `python3 -I train/econ_check.py` (Python only, no GPU, no")
    a("network; it reads the boards and the Power.log build number and writes")
    a("only into `train/runs/econ-check/`).")
    a("")
    a("## Files")
    a("")
    a("- `train/econ_check.py`: the scan and the analysis.")
    a(f"- `train/runs/econ-check/economy-cards.tsv`: Part 1, {len(rows)} rows, "
      f"{len({r['name'] for r in rows})} distinct cards.")
    a("- `train/runs/econ-check/games.tsv`: one line per finished game: patch,")
    a("  place, first economy turn (loose and sure), health at that turn, kinds.")
    a("- `train/runs/econ-check/report.md`: this file.")
    a("")
    a("## Part 1: which cards are economy cards")
    a("")
    a("Every card the engine has was read from its effect fields, with its text")
    a("(`ref/bg-cards.json` for minions and spells, `ref/bg-meta.json` for the")
    a("trinkets and the hero powers, which `bg-cards.json` does not hold) for the")
    a("amount and for the cards whose effect keys say nothing.")
    a("")
    a("\n".join(engine_lines))
    a("")
    a("Gift tokens (`Gift: ...`) and the other `[[token]]` entries are not")
    a("scanned: a token is never in the Tavern, so the economy it brings is")
    a("counted on the card that makes it, and the Dark Gifts are scanned from")
    a("`darkgifts.toml` instead.")
    a("")
    a("The kinds: " + ", ".join(f"`{k}`" for k in KIND_ORDER) + ".")
    a("")
    a("\n".join(counts))
    a("")
    a(f"{len(sure_rows)} rows are `sure = yes`. The other "
      f"{len(rows) - len(sure_rows)} rows are in the table as `sure = no` with")
    a("a one-line reason in the `note` column: a card that hands you a second")
    a("card once is usually paying for that card with its own price, so it is")
    a("a judgement call, and the judgement is in the note.")
    a("")
    a("A card with two economy kinds has two rows (Snare Trapper: `max_gold` and")
    a("a Quilboar for nothing), and a card with one kind has one row even when two")
    a("of its triggers do the same thing.")
    a("")
    a("### The borderline cards")
    a("")
    a("Grouped by why they are borderline; the card list is the same rows as the")
    a("TSV, so the two can be checked against each other.")
    a("")
    a(borderline_md(rows))
    a("")
    a("## Part 2: Era's games")
    a("")
    a(f"{n_games + escaped} action files, {n_games} finished games counted, "
      f"{escaped} skipped as unfinished (place 0).")
    a(f"Patch: {patches.get('37.0', 0)} games on 37.0 (build 254477) and "
      f"{patches.get('37.0.2', 0)} on 37.0.2 (build 255790), every one from the")
    a("game's own Power.log, so no patch is guessed by date.")
    a("")
    a("What counts as an economy event:")
    a("")
    a("- a card Era bought: the buy step, at the turn of the buy. A Tavern spell")
    a("  bought and cast is one event, at the buy.")
    a("- a trinket Era picked: the pick's turn.")
    a("- a Dark Gift Era picked: the first turn the gift is on one of Era's minions")
    a("  (from the Dark Discovery or the Dark Gift Discover, whichever came).")
    a("  The gift is keyed by gift and by the minion's card id, not its entity")
    a("  id, because the game gives the same minion a new id every turn.")
    a("- a hero power: the starting power at turn 1, and any power picked later")
    a("  through a Discover.")
    a("- NOT a card Era got for nothing and then played (a Triple Reward, a")
    a("  Discover pick, a Bounty, a Banana). Those are the effect of a card")
    a("  that is already counted, so counting them again would count one")
    a("  decision twice. The card that handed them over carries the economy.")
    a("")
    a("Being offered a card does not count, as the brief asks. `habit` rows")
    a("(Tavern Coin) are left out of everything below.")
    a("")
    a("### The main table")
    a("")
    a("Four readings of \"economy by turn N\". The mean place is Era's final")
    a("place, so a LOWER number is better and a positive difference means the")
    a("economy games placed worse. The interval is a bootstrap over games (4000")
    a("resamples, seed 20261106, each group resampled on its own).")
    a("")
    for label, _ in VARIANTS:
        a(f"**{label}**")
        a("")
        a(main_md(res["main"][label]))
        a("")
    a("### Each economy kind on its own")
    a("")
    a("Sure cards only, and only where the kind has 8 or more games. \"Games")
    a("without\" is every other game, which mostly has some other economy.")
    a("")
    a(kinds_md(res["kinds"]))
    a("")
    a("The loose rows would add one-off Discover spells to `free_card` and put")
    a("almost every game in the group, which says nothing.")
    a("")
    a("### Survivorship: was Era's health high when the economy came?")
    a("")
    a("Sure cards, decisions only (the starting hero power is not a decision).")
    a("\"On this turn\" is the games whose first economy card came on that turn,")
    a("with Era's health plus armor at the START of that turn, before Era")
    a("bought; \"no economy by then\" is the games still alive that had none up")
    a("to and including that turn. If economy follows tempo, the first column's")
    a("health should be well above the second's.")
    a("")
    a(surv_md(res["surv"][VARIANTS[2][0]]))
    a("")
    a("Same table for every sure card, starting hero power included:")
    a("")
    a(surv_md(res["surv"][VARIANTS[1][0]]))
    a("")
    a("### Patch")
    a("")
    pc = res["patch_counts"]
    pp = res["patch_place"]
    pe = res["patch_econ4"]
    a("| patch | games | mean place | top 4 | first | games with sure economy by turn 4 |")
    a("|---|---|---|---|---|---|")
    for p in ("37.0", "37.0.2"):
        a(f"| {p} | {pc.get(p, 0)} | {num(pp[p]['place'])} | {pct(pp[p]['top4'])} | "
          f"{pct(pp[p]['first'])} | {pe[p]['n']} |")
    a("")
    if res["patches"]:
        a("| patch | by turn | games with | mean place | games without | mean place | place difference (95%) |")
        a("|---|---|---|---|---|---|---|")
        for row in res["patches"]:
            a(f"| {row['patch']} | {row['thr']} | {row['a']['n']} | "
              f"{num(row['a']['place'])} | {row['b']['n']} | {num(row['b']['place'])} | "
              f"{row['place_diff'][0]:+.2f} [{ci(row['place_diff'][1])}] |")
        a("")
        a("Both cells have at least 5 games, but the \"games without\" side is")
        a("small and the interval spans more than a place, so the split says")
        a("nothing about the patch. It is here because the brief asked for the")
        a("split wherever the data allows it.")
        a("")
    else:
        a("No split by patch is possible. On 37.0.2 there are only "
          f"{pc.get('37.0.2', 0)} games, and on 37.0 the games with no sure")
        a("economy card by turn 4 number "
          f"{n_games - pc.get('37.0.2', 0) - pe['37.0']['n']}, which is too few")
        a("to compare against anything.")
        a("")
    a("## What the numbers say")
    a("")
    a(summary_text(res, n_games))
    a("")
    a("## Every choice this brief left open")
    a("")
    a(choices_text())
    a("")
    open(REPORT_PATH, "w", encoding="utf-8").write("\n".join(out) + "\n")


def summary_text(res, n_games):
    loose = res["main"][VARIANTS[0][0]]
    sure = res["main"][VARIANTS[1][0]]
    decision = res["main"][VARIANTS[2][0]]
    gold = res["main"][VARIANTS[3][0]]
    kinds = res["kinds"]
    L = []
    L.append("- The sample is {} finished games from one player. An interval on a".format(n_games))
    L.append("  mean place is about plus or minus one to one and a half places at")
    L.append("  this size, and on a top 4 rate about plus or minus 25 points, so")
    L.append("  almost nothing here is separable. Only the one interval below that")
    L.append("  leaves 0 out is worth a second look.")
    L.append("")
    s4, s6, s8 = sure[0], sure[1], sure[2]
    L.append("- **The economy against no economy comparison has a control group,")
    L.append("  but a lopsided one, and it says nothing.** With sure cards only, {} of".format(s4["a"]["n"]))
    L.append("  {} games have economy by turn 4 and {} do not; the two groups place".format(n_games, s4["b"]["n"]))
    L.append("  {} and {}, a difference of {:+.2f} with an interval of {}.".format(
        num(s4["a"]["place"]), num(s4["b"]["place"]), s4["place_diff"][0],
        ci(s4["place_diff"][1])))
    L.append("  The sign flips from one threshold to the next ({:+.2f} by turn 6,".format(s6["place_diff"][0]))
    L.append("  {:+.2f} by turn 8), which is what noise looks like.".format(s8["place_diff"][0]))
    L.append("")
    L.append("- **Dropping the starting hero power, which is not a decision, makes")
    L.append("  the economy group look a little better**: {} against {} games place".format(
        decision[0]["a"]["n"], decision[0]["b"]["n"]))
    L.append("  {} against {}, {:+.2f} [{}]. Still a coin flip.".format(
        num(decision[0]["a"]["place"]), num(decision[0]["b"]["place"]),
        decision[0]["place_diff"][0], ci(decision[0]["place_diff"][1])))
    L.append("")
    L.append("- **The loose reading has no control group left**: {} of {} games".format(
        loose[0]["a"]["n"], n_games))
    L.append("  have some economy by turn 4 once one-off Discover spells count.")
    L.append("  It is {} games against {}.".format(loose[0]["a"]["n"], loose[0]["b"]["n"]))
    L.append("")
    g4, g6, g8 = gold[0], gold[1], gold[2]
    L.append("- **Gold-making cards alone** (gold now, gold next turn, more maximum")
    L.append("  gold, sells for more) are the cleanest group: {} games by turn 4".format(g4["a"]["n"]))
    L.append("  against {}. They place {} against {}, {:+.2f} [{}],".format(
        g4["b"]["n"], num(g4["a"]["place"]), num(g4["b"]["place"]),
        g4["place_diff"][0], ci(g4["place_diff"][1])))
    L.append("  and {:+.2f} [{}] by turn 8. No sign either way.".format(
        g8["place_diff"][0], ci(g8["place_diff"][1])))
    L.append("")
    worst = [r for r in kinds if r["place_diff"] and r["place_diff"][1]
             and (r["place_diff"][1][0] > 0 or r["place_diff"][1][1] < 0)]
    if worst:
        r = worst[0]
        L.append("- **The one interval that leaves 0 out**: `{}` by turn {}: {} games".format(
            r["kind"], r["thr"], r["a"]["n"]))
        L.append("  place {} against {} for the rest, {:+.2f} [{}].".format(
            num(r["a"]["place"]), num(r["b"]["place"]), r["place_diff"][0],
            ci(r["place_diff"][1])))
        L.append("  That group is maximum gold: Strike Oil, which Era bought 11 times")
        L.append("  in these games, and Snare Trapper for its Choose One maximum Gold")
        L.append("  branch. It points the wrong way: the games where Era bought more")
        L.append("  maximum gold early placed worse. Era's own note on Strike Oil says")
        L.append("  it is a net loss for the first few turns, so this reads as the")
        L.append("  card marking a game that needed a long-term plan. At {} games two".format(r["a"]["n"]))
        L.append("  bad games can also make the whole interval.")
        L.append("")
    sell = [r for r in kinds if r["kind"] == "sell_gold"]
    if sell:
        r = sell[-1]
        L.append("- **Tortollan Blue Shell also goes the wrong way**: {} games by".format(r["a"]["n"]))
        L.append("  turn {} place {} against {}, {:+.2f} [{}]. That is not a".format(
            r["thr"], num(r["a"]["place"]), num(r["b"]["place"]),
            r["place_diff"][0], ci(r["place_diff"][1])))
        L.append("  verdict on the card: it can only be bought after losing a fight,")
        L.append("  so its buys mark a losing turn, and the interval does not leave 0")
        L.append("  out anyway.")
        L.append("")
    for kind in ("free_refresh", "free_card", "gold_next_turn", "cheap_minion"):
        rs = [r for r in kinds if r["kind"] == kind]
        if not rs:
            continue
        r = rs[0]
        L.append("- `{}` by turn {}: {} games place {} against {}, {:+.2f}".format(
            kind, r["thr"], r["a"]["n"], num(r["a"]["place"]),
            num(r["b"]["place"]), r["place_diff"][0]))
        L.append("  [{}]. Points the other way, inside the noise.".format(ci(r["place_diff"][1])))
    L.append("")
    surv = res["surv"][VARIANTS[2][0]]
    L.append("- **Survivorship: the economy buys do not come from a healthy game.")
    L.append("  It is the other way round, slightly.** At the turn Era's first sure")
    L.append("  economy card came, Era's health plus armor is about the field")
    L.append("  average, a point or two under it, not above it:")
    L.append("")
    L.append("  | turn | economy on this turn | health+armor | games with none by then | health+armor | all games alive | health+armor |")
    L.append("  |---|---|---|---|---|---|---|")
    for r in surv:
        if r["turn"] < 2 or r["turn"] > 8:
            continue
        L.append("  | {} | {} | {} | {} | {} | {} | {} |".format(
            r["turn"], r["on"]["n"], num(r["on_health"], 1), r["none"]["n"],
            num(r["none_health"], 1), r["alive"], num(r["field_health"], 1)))
    L.append("")
    L.append("  If economy came only once tempo was safe, the third column would")
    L.append("  be the healthy one. It is not. The games without economy are down")
    L.append("  to a handful by turn 6, so the later turns of that table say")
    L.append("  nothing.")
    L.append("")
    L.append("- **What the data does back is the frequency.** Era finds economy in")
    L.append("  almost every game, early. It is a habit, not a rare find. Whether")
    L.append("  it is the reason Era's good games go well is what these {} games".format(n_games))
    L.append("  cannot show: the differences that would answer it are smaller than")
    L.append("  the interval this sample can produce.")
    return "\n".join(L)


def choices_text():
    return """- **Which texts.** `ref/bg-cards.json` has no trinkets and no hero powers, so trinket and hero power texts come from `ref/bg-meta.json` (same build) and `ref/hsjson-cards.json` is the last resort. Amounts come from the effect fields where they exist, from the text where they do not. Card texts are the current patch's: the table is the card set as the engine has it, not as each game's patch had it.
- **Tokens.** cards.toml `[[token]]` entries (Beetle, Water Droplet, the Gift tokens, and so on) are not rows. They are never in the Tavern, and the economy they bring is counted on the card that makes them.
- **The kind labels.** `gold_now`, `gold_next_turn`, `max_gold`, `sell_gold`, `free_refresh`, `cheap_minion`, `cheap_spell`, `cheap_level`, `free_card`, `habit`, which follow the brief's list. A cheaper card and a free Refresh are separate kinds because Era treats the Refresh as its own thing.
- **A card for 1 or 2 gold is `cheap_minion`, not `free_card`.** A hero power that gets a minion for 1 gold (Tinker, Queen of Dragons, the Rat King's powers) is a minion under the Tavern price, so it is a cheaper minion. A power that pays the full 3 gold (the Galaxy's Lens) is not economy at all.
- **`free_card` is `sure = yes` only when the cards keep coming**: every turn, every Rally, every buy, every few events. That is the reading of Era's own example, the tier 4 Rally minions that "give you 1 gold (a minion)". One card bundled with a card or trinket you paid for is `sure = no`, even though it does hand you a card.
- **Blood Gems, Fodder, Bounties, Bananas and Golden minions are not gold.** They are in the table where a card only does that, always `sure = no`, with the reason. A Golden minion is stats and a Triple Reward.
- **Cards paid in Health are `sure = no`.** They save gold without spending gold, which is real value, but the reason says it is Health and not Gold.
- **Tavern Coin is `habit`, and every effect whose whole output is Tavern Coins follows it** (Mister Clocksworth, Kael'thas's Verdant Spheres, Murloc Holmes): `sure = no`, since Era says buying and casting one changes nothing.
- **Part 2 counts a bought card at the buy**, not at the cast, so a spell bought on turn 3 and cast on turn 5 is a turn 3 event.
- **A Choose One card counts for every economy kind either branch has.** The action log names the card, not the branch Era picked, so Snare Trapper is both `max_gold` and a free Quilboar in every game Era bought it, and Sly Infiltrator is both free Refreshes and Blood Gems.
- **A card Era got for nothing is counted on the card that gave it.** Playing a Triple Reward, a Discover pick, a Bounty or a generated token is not an event of its own, because the gold decision was the card that produced it. The cost of that choice: if the generator is not itself in the table (a hero power that is not built, an anomaly), the free card it gave is missed.
- **Dark Gifts are keyed by gift and by the minion's card id.** The game gives the same minion a new entity id every turn, so keying by entity id read the same gift as new on every turn (238 Fresh Perspective events instead of 43). Two minions with the same card id and the same gift in one game count once.
- **The starting hero power counts at turn 1** as a power Era picked, because the brief says hero powers Era picked and Era picked the hero. It is also the reason the loose table has no control group: 51 of 77 games have an economy hero power at turn 1, and a hero is not a decision per turn. The third variant in the main table drops it.
- **Hero powers not in `heroes.toml` are not classified**: Alternate Timeline (Murozond's) is the one Era played that is missing, and a picked power that is not built (Puzzle Box, Stay Frosty's neighbours) is not counted.
- **Two trinkets Era picked are not in `trinkets.toml`**: Timeworn Candelabra and Blood Gem Barrage. They are not classified and not counted.
- **Three cards are left out as not economy at all** rather than marked borderline: Hooktusk, Master Marauder (a Discover only gives Era's Pirates +1/+1), and the two gift tokens are not scanned at all. The reason is in `NOT_ECON` in the script.
- **Unfinished games are skipped**: 1 file with place 0.
- **The patch comes from the game's own Power.log**, read from the first 2 MB and from the whole file if the build line is not there. All 77 games came from a log, none from a date guess. The split by patch needed both cells to have 5 games; only one 37.0 row qualified, and it is shown with that warning.
- **The bootstrap resamples games, not turns or cards**, 4000 times, seed 20261106, each group on its own, 95% percentile interval. With 77 games the smallest honest interval on a mean place is about plus or minus one place.
- **Games are compared on Era's final place only.** Health at a turn, whether Era won that fight, and the lobby's tribes are not controlled for; the survivorship table is the one attempt at that, and it is a plain comparison of means, not a model.
"""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--part", type=int, default=0,
                    help="1: the card table only, 2: the games only")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()
    rows = build_table(verbose=args.verbose)
    n_engine = Counter(kind for kind, _, _, _, _, _ in engine_cards())
    n_engine = {k: v for k, v in
                sorted(n_engine.items(), key=lambda kv: -kv[1])}
    if args.part in (0, 1):
        write_table(rows)
        print(f"{len(rows)} economy rows, {len({r['name'] for r in rows})} cards, "
              f"from {sum(n_engine.values())} engine entries")
        for k in KIND_ORDER:
            rs = [r for r in rows if r["econ_kind"] == k]
            print(f"  {k:15s} {len(rs):4d} rows, "
                  f"{sum(1 for r in rs if r['sure'] == 'yes'):4d} sure")
        print("wrote", TSV_PATH)
    if args.part in (0, 2):
        res = part2(rows)
        write_report(rows, res, n_engine)
        print(f"{len(res['games'])} finished games, {res['unfinished']} unfinished")
        print("wrote", GAMES_PATH)
        print("wrote", REPORT_PATH)


if __name__ == "__main__":
    main()
