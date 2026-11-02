"""Check the action files of boards/actions/ against themselves and print how
many turns add up.

    ~/projects/bg/.venv/bin/python tools/check_actions.py [--verbose] [PATTERN]

Each file holds one game and, per turn, the state I started with, every step I
took and the state I ended with (docs/real-boards.md, "The action file"). The
newer keys (a step's `after`, the player-level `counters`, `secrets`, a power's
`waiting`, a minion's `reanimate`, the game's `tribes`) are for the shop replay
and are not read here: this checker still adds up Gold, Health and the cards,
with nothing from our engine. Two things are checked for every turn:

- **The Gold.** What every step took (`paid.gold`, plus any Gold the game took
  with no step of mine, `late_gold`) and gave back (a sell), against the Gold
  the turn started and ended with. A turn with Health paid for a card is
  checked the same way against Health and Armor together.
- **The cards.** My hand and board at the start, plus everything a step says it
  moved, brought in (`entered`, a Discover's pick, a buy, a steal out of the
  Tavern) or took away (`left`, a sell, a play), against my hand and board at
  the end. Cards are followed by the game's own entity id, which a card keeps
  as it goes from the Tavern to the hand to the board and when it turns golden.

A turn balances when both add up. After that, the checker names the steps whose
price is not the price the client printed (`cost`), because the file says what
happened instead: a card priced in Health, one whose Health a Soul Rewinder
gave back, a reroll paid from a pool of free ones, a price offset by a Gold
gain. Every one of those is read out of the file; anything the file does not
name is listed with its game, turn and card.

`--verbose` lists every turn that does not balance with all its reasons.
"""

import json
import sys
from collections import Counter
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "boards/actions"

# The steps the client puts a Gold price on: the card's or button's COST tag.
PRICED = ("buy", "roll", "level", "hero_power", "dark_discovery")


def cards(state):
    """A hand and a board as the cards in them, by the game's own entity id,
    and the number of hand cards the log does not name."""
    return {c["id"]: c for c in state["hand"] + state["board"]}, state.get("hand_hidden") or 0


def walk(turn):
    """The cards in my hand and board that the turn's steps account for, the
    hidden hand count, the cards stolen out of the Tavern, and the ones that
    came in with nothing named behind them."""
    have, hidden = cards(turn["start"])
    unnamed = []
    for i, a in enumerate(turn["actions"]):
        ent = id_of(a)
        mv = a.get("moved")
        if mv and ent is not None:
            to = mv.get("to")
            if to and to["where"] in ("hand", "board"):
                # a pick the game landed as a new entity says which one it made
                ent = to.get("id", ent)
                card = a.get("card") or a.get("target") or a.get("picked") or {}
                have.setdefault(ent, {"id": ent, "card_id": card.get("card_id"),
                                      **({"position": to["position"]} if "position" in to
                                         else {"index": to.get("index")})})
            else:
                have.pop(ent, None)
        for c in a.get("entered") or ():
            have[c["id"]] = c
            if not c.get("from") and not c.get("stolen"):
                unnamed.append((i, a["step"], c["card_id"]))
        for c in a.get("left") or ():
            have.pop(c["id"], None)
        if "hand_hidden" in a:
            hidden = a["hand_hidden"]
    return have, hidden, unnamed


def id_of(a):
    """The entity a step acted on: the card it moved, played, took or was aimed
    at."""
    return ((a.get("moved") or {}).get("id") or (a.get("card") or {}).get("id")
            or (a.get("target") or {}).get("id") or (a.get("picked") or {}).get("id"))


def name(card):
    """A card of a step as `card id (name)`."""
    if not card or not card.get("card_id"):
        return "no card"
    return f"{card['card_id']} ({card.get('name')})"


def ledger(turn):
    """What the turn's steps took, in Gold and in Health, and what the client's
    own numbers say the turn took."""
    spent = sum(a["paid"]["gold"] + a.get("late_gold", 0) for a in turn["actions"])
    health = sum(a["paid"]["health"] for a in turn["actions"])
    start, end = turn["start"], turn["end"]
    # a Health price is paid out of Armor first, so the two go together
    return (spent, start["gold"] - end["gold"], health,
            start["health"] + start.get("armor", 0) - end["health"] - end.get("armor", 0))


def features(turn):
    """Everything the turn's steps name: what a price was paid in, what gave
    Gold or Health back, where a free reroll came from, what the Tavern gave
    up. Each as (what it is, the card or slot it says)."""
    out = []
    for a in turn["actions"]:
        gain = a.get("gold_gain")
        if gain and gain.get("gold"):
            out.append(("a Gold gain inside a step", name(gain.get("by"))))
        if a.get("rewound"):
            out.append(("a price paid in Health, given back", name(a["rewound"].get("by"))))
        elif a.get("cost") and a["paid"]["health"]:
            out.append(("a price paid in Health", name(a.get("target") or a.get("card"))))
        if a.get("free_from"):
            out.append(("a free reroll from free Refreshes",
                        ", ".join(name(c) for c in a["free_from"])))
        for c in a.get("entered") or ():
            if c.get("stolen"):
                out.append(("a card stolen out of the Tavern", name(a.get("card"))))
    return out


def unexplained(turn):
    """The steps whose price is not what the client printed and that the file
    does not explain, and the cards that came in with nothing behind them."""
    out = []
    for a in turn["actions"]:
        cost, paid = a.get("cost"), a["paid"]
        note = f"{a['step']} priced {cost}, paid {paid['gold']} Gold and {paid['health']} Health"
        if a["step"] == "sell":
            if paid["gold"] >= 0:
                out.append(note)
            continue
        if a["step"] not in PRICED or not cost or cost == paid["gold"]:
            continue
        gain = (a.get("gold_gain") or {}).get("gold") or 0
        if cost == paid["gold"] + gain or a.get("rewound") or paid["health"]:
            continue
        if a.get("free_from") or a.get("free_refreshes"):
            continue
        out.append(note)
    return out


def check(path):
    """One file: the turns that balance, the ones that do not, and how the
    steps name what their price tags do not."""
    rec = json.loads(path.read_text())
    ok, bad = 0, []
    counts, named, sources, unnamed = Counter(), Counter(), Counter(), []
    for turn in rec["turns"]:
        counts["turns"] += 1
        reasons = []
        if turn["end"] is None:
            reasons.append("the log ends before the fight")
            counts["no end"] += 1
        else:
            spent, delta, health, hurt = ledger(turn)
            if spent != delta:
                reasons.append(f"Gold: the steps took {spent}, the turn lost {delta}")
                counts["Gold"] += 1
            if health != hurt:
                reasons.append(f"Health: the steps took {health}, the turn lost {hurt}")
                counts["Health"] += 1
            for kind, by in features(turn):
                named[kind] += 1
                if by:
                    sources[(kind, by)] += 1
            for note in unexplained(turn):
                unnamed.append((turn["turn"], note))
                counts["a price with nothing behind it"] += 1
        have, hidden, no_card = walk(turn)
        for i, step, card_id in no_card:
            unnamed.append((turn["turn"], f"{step} brought in {card_id} with nothing behind it"))
            counts["a card with nothing behind it"] += 1
        if turn["end"] is not None:
            end, end_hidden = cards(turn["end"])
            missing = [c for cid, c in end.items() if cid not in have]
            extra = [c for cid, c in have.items() if cid not in end]
            if missing or extra or hidden != end_hidden:
                reasons.append("cards: " + "; ".join(
                    [f"{c['card_id']} in my {'board' if 'position' in c else 'hand'} at the end"
                     f" that no step brought in" for c in missing] +
                    [f"{c['card_id']} gone from my {'board' if 'position' in c else 'hand'}"
                     f" that no step took" for c in extra] +
                    ([f"{end_hidden - hidden:+d} hidden hand cards"] if hidden != end_hidden else [])
                ))
                counts["cards"] += 1
        if reasons:
            bad.append((turn["turn"], reasons))
        else:
            ok += 1
    return ok, bad, counts, named, sources, unnamed


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    verbose = "--verbose" in sys.argv
    pattern = args[0] if args else "*-g*.json"
    files = sorted(OUT.glob(pattern))
    turns = ok = 0
    counts, named, sources, unnamed = Counter(), Counter(), Counter(), []
    bad_turns = []
    for path in files:
        good, bad, c, n, s, u = check(path)
        turns += c["turns"]
        ok += good
        counts.update({k: v for k, v in c.items() if k != "turns"})
        named.update(n)
        sources.update(s)
        unnamed += [(path.name, t, why) for t, why in u]
        print(f"{path.name}: {good}/{c['turns']} turns balance"
              + (f", {'does' if len(bad) == 1 else 'do'} not: "
                 + ", ".join(f"turn {t}" for t, _ in bad) if bad else ""))
        if verbose:
            for turn, reasons in bad:
                print(f"   turn {turn}:")
                for r in reasons:
                    print(f"      {r}")
            for turn, why in u:
                print(f"   turn {turn}: unnamed: {why}")
        bad_turns += [(path.name, t, r) for t, r in bad]
    print()
    print(f"{len(files)} files, {turns} turns, {ok} balance"
          + (f" ({ok / turns:.1%})" if turns else ""))
    print("what did not balance:", dict(counts) or "nothing")
    print("what the steps name instead of a price or a creator:")
    for kind, count in named.most_common():
        print(f"  {kind}: {count}")
        rows = [(n, by) for (k, by), n in sources.most_common() if k == kind]
        for n, by in rows[:8]:
            print(f"      {n:5d}  {by}")
        if len(rows) > 8:
            print(f"      and {len(rows) - 8} more kinds of source")
    print(f"left unnamed: {len(unnamed)}")
    for f, turn, why in unnamed:
        print(f"  {f} turn {turn}: {why}")
    if bad_turns and not verbose:
        print("\nthe turns that do not balance:")
        for name_, turn, reasons in bad_turns:
            print(f"  {name_} turn {turn}: " + "; ".join(reasons[:3]))


if __name__ == "__main__":
    main()
