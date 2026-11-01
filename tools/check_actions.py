"""Check the action files of boards/actions/ against themselves and print how
many turns add up.

    ~/projects/bg/.venv/bin/python tools/check_actions.py [--verbose] [PATTERN]

Each file holds one game and, per turn, the state I started with, every step I
took and the state I ended with (docs/real-boards.md, "The action file"). Two
things are checked for every turn, with nothing from our engine:

- **The Gold.** What every step took (`paid.gold`, plus any Gold the game took
  with no step of mine, `late_gold`) and gave back (a sell), against the Gold
  the turn started and ended with. A turn with Health paid for a card is
  checked the same way against Health.
- **The cards.** My hand and board at the start, plus everything a step says it
  moved, brought in (`entered`, a Discover's pick, a buy) or took away (`left`,
  a sell, a play), against my hand and board at the end. Cards are followed by
  the game's own entity id, which a card keeps as it goes from the Tavern to
  the hand to the board and when it turns golden.

A turn balances when both add up. On top of that, every step the client puts a
price on (`cost`, the card's or button's COST tag) is compared with what it
actually took; that is a diagnostic, not a balance: a card can be free, priced
in Health, or carry a COST tag that is not the Tavern's price.

`--verbose` lists every turn that does not balance with all its reasons. The
default lists each one with the first reasons, and counts the classes.
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
    hidden hand count, and the cards that came in with no card named behind
    them."""
    have, hidden = cards(turn["start"])
    unnamed = []        # (step number, step, card id) with no card behind it
    for i, a in enumerate(turn["actions"]):
        ent = id_of(a)
        mv = a.get("moved")
        if mv and ent is not None:
            to = mv.get("to")
            if to and to["where"] in ("hand", "board"):
                card = a.get("card") or a.get("target") or a.get("picked") or {}
                have.setdefault(ent, {"id": ent, "card_id": card.get("card_id"),
                                      **({"position": to["position"]} if "position" in to
                                         else {"index": to.get("index")})})
            else:
                have.pop(ent, None)
        for c in a.get("entered") or ():
            have[c["id"]] = c
            if not c.get("from"):
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


def ledger(turn):
    """What the turn's steps took, in Gold and in Health, and what the client's
    own numbers say the turn took."""
    spent = sum(a["paid"]["gold"] + a.get("late_gold", 0) for a in turn["actions"])
    health = sum(a["paid"]["health"] for a in turn["actions"])
    start, end = turn["start"], turn["end"]
    # a Health price is paid out of Armor first, so the two go together
    return (spent, start["gold"] - end["gold"], health,
            start["health"] + start.get("armor", 0) - end["health"] - end.get("armor", 0))


def prices(turn):
    """Every step whose COST tag is not what it took, named by what it looks
    like: a card priced in Health, one bought for nothing, a free reroll."""
    out = []
    for a in turn["actions"]:
        cost, paid = a.get("cost"), a["paid"]
        if a["step"] == "sell":
            if paid["gold"] >= 0:
                out.append(("a sell that did not give Gold", f"sell took {paid['gold']}"))
            continue
        if a["step"] not in PRICED or not cost or cost == paid["gold"]:
            continue
        note = f"{a['step']} priced {cost}, paid {paid['gold']} Gold and {paid['health']} Health"
        if paid["health"]:
            out.append(("priced in Health", note))
        elif not paid["gold"] and a["step"] == "roll":
            out.append(("a free reroll" + (" from a pool of them" if a.get("free") else ""), note))
        elif not paid["gold"]:
            out.append((f"a free {a['step']}", note))
        else:
            out.append(("a price its COST tag does not give", note))
    return out


def check(path):
    """One file: the turns that balance, the ones that do not with their
    reasons, and the classes of everything found."""
    rec = json.loads(path.read_text())
    ok, bad, counts, price, notes = 0, [], Counter(), Counter(), []
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
            for kind, note in prices(turn):
                price[kind] += 1
                notes.append((turn["turn"], kind, note))
        have, hidden, unnamed = walk(turn)
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
        for i, kind, cid in unnamed:
            price[f"a card came in with no card behind it ({kind})"] += 1
        if reasons:
            bad.append((turn["turn"], reasons))
        else:
            ok += 1
    return ok, bad, counts, price, notes


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    verbose = "--verbose" in sys.argv
    pattern = args[0] if args else "*-g*.json"
    files = sorted(OUT.glob(pattern))
    turns = ok = 0
    counts, price = Counter(), Counter()
    bad_turns = []
    for path in files:
        good, bad, c, p, notes = check(path)
        turns += c["turns"]
        ok += good
        counts.update({k: v for k, v in c.items() if k != "turns"})
        price.update(p)
        print(f"{path.name}: {good}/{c['turns']} turns balance"
              + (f", {'does' if len(bad) == 1 else 'do'} not: "
                 + ", ".join(f"turn {t}" for t, _ in bad) if bad else ""))
        if verbose:
            for turn, reasons in bad:
                print(f"   turn {turn}:")
                for r in reasons:
                    print(f"      {r}")
        if verbose:
            for turn, kind, note in notes:
                print(f"   turn {turn}: {kind}: {note}")
        bad_turns += [(path.name, t, r) for t, r in bad]
    print()
    print(f"{len(files)} files, {turns} turns, {ok} balance"
          + (f" ({ok / turns:.1%})" if turns else ""))
    print("what did not balance:", dict(counts) or "nothing")
    print("diagnostics (these turns still balance):", dict(price) or "nothing")
    if bad_turns and not verbose:
        print("\nthe turns that do not balance:")
        for name, turn, reasons in bad_turns:
            print(f"  {name} turn {turn}: " + "; ".join(reasons[:3]))


if __name__ == "__main__":
    main()
