"""Spot-check describe.py against the engine: every legal action gets words.

Plays whole games with a simple spend-and-level policy (so the games reach the
later turns, where trinkets and tier 5 cards show up), and at every decision
checks that

- every legal index in the mask gets a description and nothing else does,
- the description of the action it is about to take names a card or a plain
  word, never a placeholder,
- the state text agrees with the observation: the Gold, tier, health, the
  hand and board counts, the Tavern cards and the lobby types it prints are
  the values the observation holds,
- the indices decode back to the engine's own layout (`N_ACTIONS`, the
  action kinds and their sizes from `ACTION_KINDS`).

It also reports which action kinds the games exercised, because a kind that
never came up is a kind whose words were not checked.

Offline, no network, no API key:

    .venv/bin/python train/llm/test_describe.py [--games N] [--seed S]
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np

import bgpy

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import describe  # noqa: E402


def obs_arrays(n=1):
    return dict(
        obs=np.zeros((n, bgpy.OBS_SIZE), np.float32),
        mask=np.zeros((n, bgpy.N_ACTIONS), bool),
        seat=np.zeros(n, np.int64),
        kind=np.zeros(n, np.int64),
        places=np.zeros((n, bgpy.PLAYERS), np.int8),
        tier_turns=np.zeros((n, bgpy.PLAYERS, bgpy.MAX_TIER), np.int8),
        tribes=np.zeros((n, bgpy.PLAYERS, bgpy.TRIBES), np.int8),
        reset=np.zeros(n, bool),
    )


def check_layout():
    """The action indices the text uses are the engine's own."""
    total = sum(size for _, size in bgpy.ACTION_KINDS)
    assert total == bgpy.N_ACTIONS, (total, bgpy.N_ACTIONS)
    seen = 0
    for kind, size in bgpy.ACTION_KINDS:
        for _ in range(size):
            assert describe.action_at(seen)[0] == kind, (seen, kind)
            seen += 1
    assert describe.action_at(bgpy.END_TURN)[0] == "end"
    assert describe.action_at(bgpy.HERO_POWER)[0] == "hero_power"
    assert describe.action_at(bgpy.HERO_POWER2)[0] == "hero_power2"
    assert describe.END_TURN == bgpy.END_TURN
    assert describe.HERO_POWER == bgpy.HERO_POWER
    assert describe.HERO_POWER2 == bgpy.HERO_POWER2


def check_state(desc, o, m):
    """The state text must say what the observation holds."""
    state, rows = desc.describe(o, m, 0)
    gold = int(round(float(o[bgpy.SCALAR_GOLD])))
    tier = int(round(float(o[bgpy.SCALAR_TIER])))
    health = int(round(float(o[bgpy.SCALAR_HEALTH])))
    turn = int(round(float(o[bgpy.SCALAR_TURN])))
    assert f"TURN {turn}" in state, turn
    assert f"Health {health}" in state, health
    assert f"Gold {gold} of " in state, gold
    assert f"Tavern tier {tier}" in state, tier

    hand = [k for k in range(bgpy.HAND) if desc.present(desc.slot(o, bgpy.HAND_START, k))]
    board = [k for k in range(bgpy.BOARD) if desc.present(desc.slot(o, bgpy.BOARD_START, k))]
    assert f"YOUR HAND ({len(hand)} of {bgpy.HAND})" in state
    assert f"YOUR BOARD ({len(board)} of {bgpy.BOARD}" in state

    # Every Tavern minion the observation holds is named in the state.
    for k in range(bgpy.SHOP):
        f = desc.slot(o, bgpy.SHOP_START, k)
        if not desc.present(f):
            continue
        name = desc.card_name(f[bgpy.SLOT_CARD_ID])
        assert f"Tavern slot {k + 1}: {name} " in state, (k, name)
        if float(f[bgpy.SLOT_IS_SPELL]) != 0.0:
            # A spell that sits in a minion slot (Saloon's Finest) has no
            # stats; the text shows its cost instead.
            assert f"{int(round(float(f[bgpy.SLOT_COST])))} Gold" in state, name
            continue
        attacked = int(round(float(f[bgpy.SLOT_ATTACK])))
        held = int(round(float(f[bgpy.SLOT_HEALTH])))
        assert f"{attacked}/{held}" in state, (name, attacked, held)

    # The lobby's tribes are the multi-hot the observation carries.
    tribes = [describe.TRIBES[j] for j in range(bgpy.LOBBY_TRIBES)
              if float(o[bgpy.SCALAR_LOBBY + j]) != 0.0]
    assert "Lobby types: " + ", ".join(tribes) in state, tribes

    # Every opponent gets a line, with the Health it holds.
    for k in range(1, bgpy.PLAYERS):
        base = bgpy.OWN_END + (k - 1) * bgpy.OPP_BLOCK
        head = f"  Opponent {k} (seat "
        assert head in state, k
        line = state[state.index(head):].splitlines()[0]
        health = max(0, int(round(float(o[base + bgpy.OPP_HEALTH]))))
        assert f"Health {health}" in line, line
        assert f"tier {int(round(float(o[base + bgpy.OPP_TIER])))}" in line, line

    # Every legal index is described exactly once, and only legal ones are.
    legal = [int(i) for i in np.nonzero(m)[0]]
    assert [i for i, _ in rows] == legal, (len(rows), len(legal))
    for index, words in rows:
        assert words and words != f"action {index}", index
        assert "None" not in words and "?" not in words, (index, words)

    # End turn is legal in the shop phase: it is only missing while a choice
    # (a target, a Discover, a hero or power pick) is waiting for its answer.
    choice_open = any(
        float(o[i]) != 0.0 for i in (
            bgpy.SCALAR_PENDING_TARGET, bgpy.SCALAR_PENDING_TARGET_SHOP,
            bgpy.SCALAR_PENDING_SHOP_SLOT, bgpy.SCALAR_PENDING_CHOOSE_ONE,
            bgpy.SCALAR_PENDING_DISCOVER, bgpy.SCALAR_PENDING_DISCARD,
            bgpy.SCALAR_PENDING_HERO, bgpy.SCALAR_PENDING_POWER,
            bgpy.SCALAR_PENDING_WAGER, bgpy.SCALAR_PENDING_HOLMES,
        )
    ) or any(float(o[describe.SCALAR_SECRET_OFFER + k]) != 0.0
             for k in range(bgpy.CHOICES))
    if not choice_open:
        assert bgpy.END_TURN in legal, "no end turn in the shop phase"
        assert "end turn" in dict(rows)[bgpy.END_TURN]
    return state, rows


def check_texts(desc):
    """The reference texts really are reachable by the names the engine uses."""
    plain = desc.card_text("Buzzing Vermin")
    golden = desc.card_text("Buzzing Vermin", True)
    assert plain and golden and plain != golden, (plain, golden)
    assert "two 2/2 Beetles" in golden, golden
    assert "Taunt" in desc.card_text("Blood Gem (Taunt)")
    assert "Divine Shield" in desc.card_text("Blood Gem (Divine Shield)")
    assert desc.card_text("Dark Paradox t3")
    assert desc.gift_text("Gift: Fortitude")
    assert desc.gift_text("Gift: Battle Scars strong") != desc.gift_text("Gift: Battle Scars")
    blank = [n for n in desc.card_names if not desc.card_text(n, False)]
    # Only summoned tokens have no printed text in the data.
    assert len(blank) <= 20, blank
    # The tables describe.py builds from the .toml files must be the engine's
    # own, in its order, or an index from the observation names the wrong card.
    assert len(desc.powers) == bgpy.N_POWERS, (len(desc.powers), bgpy.N_POWERS)
    assert len(desc.trinkets) == bgpy.N_TRINKETS, (len(desc.trinkets), bgpy.N_TRINKETS)
    assert len(desc.hero_names) == bgpy.N_HEROES
    assert len(desc.card_names) == bgpy.N_CARDS
    for i in range(1, len(desc.powers) + 1):
        name, _text = desc.power_text(i)
        assert name and name != "?", i
    for i in range(1, len(desc.trinkets) + 1):
        name, text = desc.trinket_name(i)
        assert name and name != "?" and text, i


def play_game(desc, rng, max_decisions=400):
    arrays = obs_arrays()
    v = bgpy.VecEnv(1, int(rng.integers(1, 10**6)), [1, 0, 0, 7, 0, 0])
    checks = 0
    kinds = set()
    last_turn = 0
    for _ in range(max_decisions):
        v.observe(arrays["obs"], arrays["mask"], arrays["seat"], arrays["kind"])
        o, m = arrays["obs"][0], arrays["mask"][0]
        state, rows = check_state(desc, o, m)
        checks += 1
        last_turn = int(round(float(o[bgpy.SCALAR_TURN])))
        # The chosen action must describe a card that is really where it says.
        legal = [i for i, _ in rows]
        action = choose(desc, o, legal, rng)
        kind, a, b = describe.action_at(action)
        kinds.add(kind)
        words = dict(rows)[action]
        if kind in ("buy", "sell", "play", "move", "activate", "target", "discard"):
            start = bgpy.SHOP_START if kind == "buy" else (
                bgpy.HAND_START if kind in ("play", "discard") else bgpy.BOARD_START)
            f = desc.slot(o, start, a)
            if desc.present(f):
                assert desc.card_name(f[bgpy.SLOT_CARD_ID]) in words, words
        del state
        v.step(np.array([action]), arrays["places"], arrays["tier_turns"],
               arrays["tribes"], arrays["reset"])
        if arrays["places"][0].any():
            break
    return checks, int(arrays["places"][0].max()), last_turn, kinds


def choose(desc, o, legal, rng):
    """Answer an open choice first, then play, buy, level, activate, end."""
    kinds = {i: describe.action_at(i)[0] for i in legal}
    forced = [i for i in legal if kinds[i] in (
        "choose", "target", "target_shop", "discard", "choose_trinket",
        "hero_power", "hero_power2", "dark_gift")]
    if forced:
        return int(rng.choice(forced))
    hand = sum(1 for k in range(bgpy.HAND) if desc.present(desc.slot(o, bgpy.HAND_START, k)))
    board = sum(1 for k in range(bgpy.BOARD) if desc.present(desc.slot(o, bgpy.BOARD_START, k)))
    for kind, ok in (("play", True), ("buy", hand < 9), ("level", True),
                     ("activate", True), ("roll", board < 7 and hand < 9)):
        picks = [i for i in legal if kinds[i] == kind]
        if picks and ok:
            return int(rng.choice(picks))
    ends = [i for i in legal if kinds[i] == "end"]
    if ends:
        return ends[0]
    return int(rng.choice(legal))


def main():
    a = argparse.ArgumentParser(description="Check describe.py against the engine.")
    a.add_argument("--games", type=int, default=2)
    a.add_argument("--seed", type=int, default=11)
    args = a.parse_args()
    check_layout()
    v = bgpy.VecEnv(1, args.seed, [1, 0, 0, 0, 7, 0])
    desc = describe.Descriptor(v)
    check_texts(desc)
    rng = np.random.default_rng(args.seed)
    total = 0
    places = []
    kinds = set()
    deepest = 0
    for g in range(args.games):
        checks, place, last_turn, seen = play_game(desc, rng)
        total += checks
        places.append(place)
        kinds |= seen
        deepest = max(deepest, last_turn)
        print(f"game {g + 1}: {checks} decisions checked, final place {place}, "
              f"last turn {last_turn}")
    assert all(1 <= p <= 8 for p in places), places
    # The kinds these games must have exercised, or the check means little.
    for kind in ("buy", "play", "level", "end"):
        assert kind in kinds, (kind, sorted(kinds))
    assert kinds & {"choose", "target", "target_shop"}, sorted(kinds)
    assert deepest >= 6, deepest
    print(f"action kinds described: {', '.join(sorted(kinds))}")
    print(f"ok: {total} decisions, every legal action described, no illegal index")


if __name__ == "__main__":
    main()
