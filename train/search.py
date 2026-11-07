"""The search: at each shop decision, try every move and every short cycle on
copies of the game, judge them, and play the best (the "Search v1" brief in
docs/briefs/). Play time only: nothing here trains anything.

A cycle is a short line that costs a little gold and leaves the position
better (`docs/notes/era-strategy.md`, "Gold-neutral loops"): buy a card, play
it, sell what the player would not keep. Era's two examples are the Tortollan
Blue Shell after a loss (+2 gold for two actions) and Brann with Kalecgos and
a Draconic Warden (two Chromadrakes for free, the Dragons buffed).

The two judges:

1. **Dominance** (exact, in Rust: `crates/bgcore/src/search.rs`): compare the
   position at the end of a cycle with the position before it. Nothing worse
   and something better means Era's "same position, only better", and the
   cycle is played at once (`Searcher` loops: after a cycle the shop may offer
   another one).
2. **Real trades** (the network plus fights): a move or a cycle that costs
   gold is played out to the end of the turn on a copy, `playouts` times with
   the network sampling its moves, and each finished turn is scored as the
   network's value of the position plus `fight_weight` times the Health the
   coming fight is expected to cost against the boards of Era's real games.

The copies are `bgpy.SearchBatch` slots: game clones with their own RNG and
pool, so nothing the search does touches the game it decides for. The action
the search picks is then played on the real game by the caller, and the rest
of the line follows as a plan (`Searcher.plans`) so the real game takes the
same steps.

What the extra flags mean (see `eval_search.py`):

- `--replan every` (the default) runs the real-trade judge at every shop
  decision, which is what the brief asks for. `turn` runs it once per turn,
  at the turn's first shop decision, and leaves the rest of the turn to the
  network with the cheap dominance cycles at every decision (about ten times
  cheaper). `cycle` never runs it (dominance only, the cheapest search).
- `--fight-weight` weights the fight's expected Health change against the
  network's value: 1 means a whole hero's Health (30 points, `HEALTH_SCALE`)
  is worth one whole step of the value, which runs from -1 (last) to +1
  (first).
"""

from __future__ import annotations

import time

import numpy as np
import torch

import bgpy
from model import masked

# The action space, by kind: `KIND_OF[a]` is the kind of action `a` and
# `FIRST["buy"]`, `FIRST["play"]`, ... are the index of its first action.
KINDS = [name for name, _ in bgpy.ACTION_KINDS]
KIND_OF = np.array(
    [k for k, (_, n) in enumerate(bgpy.ACTION_KINDS) for _ in range(n)], np.int32
)
FIRST = {name: int(np.nonzero(KIND_OF == k)[0][0]) for k, name in enumerate(KINDS)}
K_BUY = KINDS.index("buy")
K_SELL = KINDS.index("sell")
K_PLAY = KINDS.index("play")
K_CHOOSE = KINDS.index("choose")
PLAY_SPAN = bgpy.BOARD + 1
# A play action's hand slot and position, from the action index.
def play_action(hand: int, pos: int) -> int:
    return FIRST["play"] + hand * PLAY_SPAN + pos


def play_parts(a: int) -> tuple[int, int]:
    j = a - FIRST["play"]
    return j // PLAY_SPAN, j % PLAY_SPAN


# The Health a fight weight of 1 pays for: a whole hero's Health, in the
# network's value units (the value covers -1 to +1 over a whole game).
HEALTH_SCALE = 30.0
# The most a cycle may cost and still count (the brief: 2 gold).
MAX_CYCLE_COST = 2
# How many answers (a target, a Discover pick, a Choose One) one line may take
# before the search gives up on it.
MAX_ANSWERS = 8


class Slots:
    """A `bgpy.SearchBatch` and the numpy arrays bgpy fills for it."""

    def __init__(self, batch: bgpy.SearchBatch):
        self.b = batch
        self.obs = np.zeros((batch.len(), bgpy.OBS_SIZE), np.float32)
        self.mask = np.zeros((batch.len(), bgpy.N_ACTIONS), bool)
        self.state = np.zeros(batch.len(), np.int8)
        self.dom = np.zeros(batch.len(), bool)
        self.better = np.zeros(batch.len(), bool)
        self.health = np.zeros(batch.len(), np.int32)
        # The actions each slot took from the position it was copied from.
        self.lines: list[list[int]] = [[] for _ in range(batch.len())]

    def __len__(self) -> int:
        return len(self.lines)

    def ensure(self, n: int) -> None:
        """Room for n slots (the batch grows with them)."""
        if n <= len(self.lines):
            return
        if n > self.b.len():
            self.b.grow(n - self.b.len())
        grow = n - len(self.lines)
        self.obs = np.concatenate([self.obs, np.zeros((grow, bgpy.OBS_SIZE), np.float32)])
        self.mask = np.concatenate([self.mask, np.zeros((grow, bgpy.N_ACTIONS), bool)])
        self.state = np.concatenate([self.state, np.full(grow, 2, np.int8)])
        self.dom = np.concatenate([self.dom, np.zeros(grow, bool)])
        self.better = np.concatenate([self.better, np.zeros(grow, bool)])
        self.health = np.concatenate([self.health, np.zeros(grow, np.int32)])
        self.lines.extend([] for _ in range(grow))

    def sync(self) -> None:
        """Match the batch, which a caller may have grown or cut."""
        n = self.b.len()
        if n > len(self.lines):
            self.ensure(n)
        elif n < len(self.lines):
            self.truncate(n)

    def push(self, src: int) -> int:
        """A copy of slot `src`, its line included."""
        self.sync()
        slot = len(self.lines)
        self.ensure(slot + 1)
        self.b.set(slot, src)
        self.lines[slot] = list(self.lines[src])
        self.state[slot] = 2
        return slot

    def act(self, slot: int, actions) -> None:
        self.b.act(slot, list(actions))
        self.lines[slot].extend(actions)

    def observe(self) -> None:
        self.sync()
        self.b.observe_all(self.obs, self.mask, self.state)

    def describe(self, slot: int):
        return self.b.describe(slot)

    def dominance(self, base: int, after: int):
        return self.b.dominance(base, after)

    def truncate(self, n: int) -> None:
        self.b.truncate(n)
        self.lines = self.lines[:n]
        self.obs = self.obs[:n]
        self.mask = self.mask[:n]
        self.state = self.state[:n]
        self.dom = self.dom[:n]
        self.better = self.better[:n]
        self.health = self.health[:n]


class Candidate:
    """One thing the search may play: a move, or a cycle (the brief's
    candidate)."""

    __slots__ = ("actions", "slot", "label", "kind", "cost", "dominates", "better", "variant")

    def __init__(self, actions, slot, label, kind, cost=0, variant=""):
        self.actions = list(actions)
        self.slot = slot
        self.label = label
        self.kind = kind
        self.cost = cost
        self.dominates = False
        self.better = False
        self.variant = variant

    def __repr__(self) -> str:
        return f"<{self.kind} {self.label} cost {self.cost} {self.variant}>"


class Searcher:
    """The search for one network, over any number of games.

    `mode` is "search" (both judges) or "plain" (the network alone, for the
    same measurements on the same positions)."""

    def __init__(
        self,
        net,
        dev="cpu",
        mode="search",
        playouts=4,
        fights=20,
        fight_weight=0.0,
        seed=0,
        replan="every",
        max_actions=200,
        temperature=1.0,
        cycle_variants="all",
        log=None,
    ):
        self.net = net
        self.dev = dev
        self.mode = mode
        self.playouts = max(1, int(playouts))
        self.fights = int(fights)
        self.fight_weight = float(fight_weight)
        self.replan = replan
        self.max_actions = int(max_actions)
        self.temperature = float(temperature)
        self.cycle_variants = cycle_variants
        self.log = log or (lambda *a, **k: None)
        # A forced mask (ppo.force_mask), or None: the network's choices in a
        # search and the plain baseline must be forced the same way.
        self.force = None
        self.gen = torch.Generator(device="cpu")
        self.gen.manual_seed(int(seed) & 0xFFFF_FFFF)
        self.fight_seed = (int(seed) & 0xFFFF_FFFF) ^ 0x5EED_1234
        # The rest of a line already judged, per (game, seat): (turn, actions).
        self.plans: dict[tuple, tuple[int, list[int]]] = {}
        self.names: list[str] | None = None
        # Counters, for the report.
        self.decisions = 0
        self.dominance_fired = 0
        self.cycles_played = 0
        self.moves_scored = 0
        self.playout_steps = 0
        self.seconds = 0.0
        # The turn each game's last real-trade judge ran in.
        self._judged: dict[tuple, int] = {}

    # -- the network -------------------------------------------------------

    def _masked_action(self, S: Slots, rows, sample=True) -> np.ndarray:
        mask = torch.from_numpy(np.ascontiguousarray(S.mask[rows]))
        o = torch.from_numpy(np.ascontiguousarray(S.obs[rows])).to(self.dev)
        with torch.no_grad():
            logits, _ = self.net(o)
        m = mask
        if self.force is not None:
            # The eval's forced actions (a Dark Gift on turns 3 to 5), the same
            # ones ppo.py's own evaluation forces: the searcher's network
            # decisions and the plain baseline must be the same player.
            m = self.force(o, mask.to(self.dev)).cpu()
        logits = masked(logits.float().cpu(), m)
        if sample and self.temperature > 0:
            probs = torch.softmax(logits / self.temperature, -1)
            a = torch.multinomial(probs, 1, generator=self.gen).squeeze(1)
        else:
            a = logits.argmax(1)
        return a.numpy()

    # -- the decision ------------------------------------------------------

    def decide(self, env, i: int, seat: int) -> int:
        """The action for game `i`'s learner seat on the real VecEnv."""
        return self.decide_batch(env.search_copy(i), (i, seat))

    def decide_batch(self, b: bgpy.SearchBatch, key=("turn", 0)) -> int:
        """The action for one position. `b`'s slot 0 is the position; scratch
        slots are added and may be dropped with `Slots.truncate`."""
        self.decisions += 1
        t0 = time.time()
        try:
            return self._decide(b, key)
        finally:
            self.seconds += time.time() - t0

    def _decide(self, b: bgpy.SearchBatch, key) -> int:
        S = Slots(b)
        # A line already judged plays out its remaining actions.
        plan = self.plans.get(key)
        if plan is not None:
            turn, actions = plan
            if turn == b.turn and actions:
                a = actions.pop(0)
                if not actions:
                    del self.plans[key]
                return a
            del self.plans[key]
        if self.mode == "plain":
            S.observe()
            return int(self._masked_action(S, [0])[0])
        if b.bucket > 0:
            # The same boards and seeds for every candidate of this decision.
            b.set_fights(self.fights, self.fight_seed)
        pending: list[int] = []
        cycles: list[Candidate] = []
        base = 0
        while True:
            S.observe()
            if S.state[base] != 0:
                break
            cycles = self.cycles(S, base)
            dominating = [c for c in cycles if c.dominates]
            if not dominating:
                break
            # Several dominating cycles: the one item 3 scores best.
            if len(dominating) > 1:
                scores = self.score(S, dominating)
                best = dominating[int(np.argmax(scores))]
            else:
                best = dominating[0]
            self.dominance_fired += 1
            self.cycles_played += 1
            self.log(f"dominance: {best.label} {best.variant} cost {best.cost}")
            pending.extend(best.actions)
            base = best.slot
        def play(actions) -> int:
            """Hand the line back: its first action now, the rest as a plan."""
            pending.extend(actions)
            self.plans[key] = (b.turn, pending[1:])
            return pending[0]

        if S.state[base] != 0:
            # The dominance cycles ended the turn (or the game).
            if not pending:
                return int(self._masked_action(S, [base])[0])
            return play([])
        if not self.want_item3(key, b.turn):
            # The network's action comes after the cycles already played.
            return play([int(self._masked_action(S, [base])[0])])
        cands = self.moves(S, base) + cycles
        if not cands:
            return play([int(self._masked_action(S, [base])[0])])
        scores = self.score(S, cands)
        best = cands[int(np.argmax(scores))]
        self.log(f"item 3: {best.label} {best.variant} cost {best.cost} score {max(scores):.3f}")
        if best.kind == "cycle":
            self.cycles_played += 1
        return play(best.actions)

    def want_item3(self, key, turn: int) -> bool:
        if self.replan == "cycle":
            return False
        if self.replan == "every":
            return True
        # "turn": once per turn and seat, at its first shop decision.
        if self._judged.get(key) == turn:
            return False
        self._judged[key] = turn
        return True

    # -- the candidates ----------------------------------------------------

    def moves(self, S: Slots, base: int = 0) -> list[Candidate]:
        """Every legal action: the brief's first half of the candidates."""
        out = []
        desc = S.describe(base)
        for a in np.nonzero(S.mask[base])[0]:
            a = int(a)
            label = self.action_label(desc, a)
            slot = S.push(base)
            S.act(slot, [a])
            out.append(Candidate([a], slot, label, "move"))
        return out

    def action_label(self, desc, a: int) -> str:
        """What an action does, by name, for the log and the counts."""
        kind = KINDS[KIND_OF[a]]
        if KIND_OF[a] == K_BUY:
            shop = desc[8]
            return f"buy {self.name(shop[a])}" if a < len(shop) else "buy"
        if KIND_OF[a] == K_SELL:
            board = desc[9]
            return f"sell {self.name(board[a - FIRST['sell']])}" if a - FIRST["sell"] < len(board) else "sell"
        if KIND_OF[a] == K_PLAY:
            hand, pos = play_parts(a)
            hand_cards = desc[7]
            return f"play {self.name(hand_cards[hand])} at {pos}" if hand < len(hand_cards) else "play"
        return kind

    def name(self, entry) -> str:
        if self.names is None:
            return str(entry)
        cid = entry[0] if isinstance(entry, (tuple, list)) else entry
        return self.names[cid] if cid < len(self.names) else str(cid)

    def cycles(self, S: Slots, base: int = 0) -> list[Candidate]:
        """Every cycle from `base`: for each card in the shop and in hand, buy
        (from the shop), play, then sell the played card and every card the
        line generated that the player would not keep. Each cycle is one
        candidate; a line that opens a Discover is branched once per option."""
        desc = S.describe(base)
        gold = desc[0]
        hand = desc[7]
        board = desc[9]
        if len(hand) >= bgpy.HAND or len(board) >= bgpy.BOARD:
            # A cycle plays a card, which needs room in the hand for what it
            # gives back and a slot on the board.
            return []
        legal = np.nonzero(S.mask[base])[0]
        sources: list[tuple[str, int, list[int]]] = []
        for a in legal:
            a = int(a)
            if KIND_OF[a] == K_BUY:
                sources.append(("shop", a, [a]))
            elif KIND_OF[a] == K_PLAY:
                h, pos = play_parts(a)
                # One play per hand card: to the end of the board.
                if pos == len(board) and h < len(hand):
                    sources.append(("hand", a, [a]))
        out: list[Candidate] = []
        # A candidate's actions are what a line did after the position it
        # started from: slot 0's own line is the cycles already played.
        base_len = len(S.lines[base])
        for origin, src_action, prefix in sources:
            out.extend(self._one_cycle(S, base, desc, origin, src_action, prefix, gold, base_len))
        return out

    def _one_cycle(self, S: Slots, base, desc, origin, src_action, prefix, gold, base_len) -> list[Candidate]:
        """The variants of one cycle: sell everything (the dominating
        variant), the same with the generated cards played before the played
        card is sold, and the two keeps, which only item 3 judges."""
        out: list[Candidate] = []
        hand_before = desc[7]
        board_len = len(desc[9])
        label_card = self.name(desc[8][src_action]) if origin == "shop" else self.name(
            hand_before[play_parts(src_action)[0]]
        )
        label = f"cycle {label_card}"
        # The sell-the-played-card-last order needs its own line.
        for order in ("played_first", "generated_first"):
            slot = self._line_until_played(S, base, prefix, origin, desc)
            if slot is None:
                break
            ends = self._sell_all(S, slot, hand_before, board_len, order)
            for end in ends:
                d = S.describe(end)
                cost = gold - d[0]
                line = S.lines[end][base_len:]
                if cost > MAX_CYCLE_COST or not line:
                    continue
                c = Candidate(line, end, label, "cycle", cost, order)
                dom, better, _ = S.dominance(base, end)
                c.dominates, c.better = dom, better
                out.append(c)
        if self.cycle_variants == "dominating":
            return out
        # The keeps: judged by item 3 only, never by dominance.
        for keep in ("keep_generated", "keep_played"):
            slot = self._line_until_played(S, base, prefix, origin, desc)
            if slot is None:
                break
            ends = self._sell_all(S, slot, hand_before, board_len, keep)
            for end in ends:
                d = S.describe(end)
                cost = gold - d[0]
                if cost > MAX_CYCLE_COST:
                    continue
                line = S.lines[end][base_len:]
                if not line:
                    continue
                out.append(Candidate(line, end, label, "cycle", cost, keep))
        return out

    def _line_until_played(self, S: Slots, base, prefix, origin, desc):
        """Buy (when the card is in the shop) and play it, answers and all.
        Returns the slot of the position after the play, or None: the card
        cannot be played on the board the position has."""
        slot = S.push(base)
        try:
            S.act(slot, prefix)
        except Exception:
            S.truncate(slot)
            return None
        d = S.describe(slot)
        pos = len(d[9])
        hand = d[7]
        if not hand:
            S.truncate(slot)
            return None
        hand_index = len(hand) - 1 if origin == "shop" else play_parts(prefix[0])[0]
        a = play_action(hand_index, pos)
        S.observe()
        if S.state[slot] != 0 or not S.mask[slot][a]:
            S.truncate(slot)
            return None
        S.act(slot, [a])
        ends = self._settle(S, slot)
        if not ends:
            S.truncate(slot)
            return None
        # One play can open more than one answer; the sell line continues on
        # each (only a Discover branches, so there is usually one).
        return ends[0]

    def _settle(self, S: Slots, slot: int, branch: bool = True, depth: int = 2) -> list[int]:
        """Answer whatever the slot waits on (a target, a Discover, a Choose
        One), with the network's choice; a Discover is branched once per
        option when `branch`. Returns the slots that reached a plain shop
        decision, or none when the line cannot continue."""
        applied = 0
        for _ in range(MAX_ANSWERS):
            S.observe()
            state = S.state[slot]
            if state == 0:
                return [slot]
            if state != 1:
                return []
            legal = [int(a) for a in np.nonzero(S.mask[slot])[0]]
            choose = [a for a in legal if KIND_OF[a] == K_CHOOSE]
            if not choose:
                # A target, a discard, a trinket pick: the network answers.
                a = int(self._masked_action(S, [slot], sample=False)[0])
                S.act(slot, [a])
                applied += 1
                continue
            if not branch or depth <= 0 or len(choose) == 1:
                a = int(self._masked_action(S, [slot], sample=False)[0])
                S.act(slot, [a])
                applied += 1
                continue
            ends = []
            for a in choose:
                child = S.push(slot)
                S.act(child, [a])
                ends.extend(self._settle(S, child, branch=False, depth=depth - 1))
            return ends
        return []

    def _sell_all(self, S: Slots, slot: int, hand_before, board_len: int, order: str) -> list[int]:
        """Play and sell every card the line generated, and sell the played
        card, in the order `order` names. Returns the slots at the end."""
        d = S.describe(slot)
        generated = self._generated(d[7], hand_before)
        ends = [slot]
        # The played card sits at the board slot the play put it in.
        played_pos = board_len
        sell_played = order != "keep_played"
        sell_generated = order != "keep_generated"
        sold_played_first = order != "generated_first"
        if sell_played and sold_played_first:
            ends = [self._sell_at(S, e, played_pos) for e in ends]
        if not sell_generated:
            return ends
        # The generated cards: play each, answer it, sell it.
        for card in generated:
            nxt = []
            for e in ends:
                d = S.describe(e)
                hand = d[7]
                index = next((i for i, m in enumerate(hand) if m[0] == card), None)
                if index is None:
                    nxt.append(e)
                    continue
                pos = len(d[9])
                a = play_action(index, pos)
                S.observe()
                if pos >= bgpy.BOARD or not S.mask[e][a]:
                    # A card that cannot be played (a spell, a locked card):
                    # it stays in the hand, which the dominance judge reads as
                    # a gain when the hand had room.
                    nxt.append(e)
                    continue
                S.act(e, [a])
                settled = self._settle(S, e)
                for s in settled:
                    nxt.append(self._sell_at(S, s, pos))
            ends = nxt
        if sell_played and not sold_played_first:
            ends = [self._sell_at(S, e, played_pos) for e in ends]
        return ends

    def _sell_at(self, S: Slots, slot: int, pos: int) -> int:
        """Sell board slot `pos` when it can be sold; otherwise leave it."""
        S.observe()
        a = FIRST["sell"] + pos
        if S.state[slot] == 0 and pos < bgpy.BOARD and S.mask[slot][a]:
            S.act(slot, [a])
        return slot

    @staticmethod
    def _generated(hand_after, hand_before) -> list[int]:
        """The card ids the line put in the hand that were not there before,
        in order (a multiset difference)."""
        seen = list(hand_before)
        out = []
        for m in hand_after:
            match = next((i for i, x in enumerate(seen) if x[0] == m[0]), None)
            if match is None:
                out.append(m[0])
            else:
                seen.pop(match)
        return out

    # -- the second judge: playouts, fights and the value ------------------

    def score(self, S: Slots, cands: list[Candidate]) -> np.ndarray:
        """The average score of every candidate: the network's value of the
        end of the turn plus the fight weight times the expected Health the
        coming fight costs."""
        P = self.playouts
        rows: list[int] = []
        for c in cands:
            for _ in range(P):
                slot = S.push(c.slot)
                S.b.mark(slot)
                rows.append(slot)
        # The rest of the turn, step by step, for every playout at once.
        for _ in range(self.max_actions):
            S.observe()
            active = [s for s in rows if S.state[s] == 0]
            if not active:
                break
            a = self._masked_action(S, active, sample=True)
            actions = np.full(len(S.lines), -1, np.int64)
            for s, x in zip(active, a):
                actions[s] = int(x)
            S.b.step_all(actions)
            self.playout_steps += 1
            # An improving step along the way is taken: the mark moves with it.
            S.b.dominating(S.dom[: len(S.lines)], S.better[: len(S.lines)])
        end = np.zeros(len(S.lines), bool)
        end[rows] = True
        S.b.end_all(end)
        S.observe()
        self.moves_scored += len(cands)
        obs = np.ascontiguousarray(S.obs[rows])
        o = torch.from_numpy(obs).to(self.dev)
        with torch.no_grad():
            _, value = self.net(o)
        value = value.float().cpu().numpy().reshape(-1)
        health = S.health[: len(S.lines)]
        if self.fights > 0:
            S.b.fights(health)
        health = health[rows].astype(np.float64)
        boards = max(1, S.b.n_fights)
        score = value + self.fight_weight * health / (HEALTH_SCALE * boards)
        return score.reshape(len(cands), P).mean(1)

    # -- counters ----------------------------------------------------------

    def stats(self) -> dict:
        return {
            "decisions": self.decisions,
            "dominance_fired": self.dominance_fired,
            "cycles_played": self.cycles_played,
            "candidates_scored": self.moves_scored,
            "playout_steps": self.playout_steps,
            "seconds": self.seconds,
            "seconds_per_decision": self.seconds / max(1, self.decisions),
        }


class TurnPlay:
    """Plays one seat's shop turn on a batch and says what it did: the cards
    bought, played and sold, and which of them were cycles (a card bought or
    played and sold the same turn), the cycle-rate test's measure."""

    def __init__(self, batch: bgpy.SearchBatch, searcher: Searcher, names=None):
        self.b = batch
        self.searcher = searcher
        self.names = names
        self.actions: list[int] = []
        self.bought: list[int] = []
        self.played: list[int] = []
        self.sold: list[int] = []
        self.gold_start = 0
        self.gold_end = 0

    def name(self, cid: int) -> str:
        return self.names[cid] if self.names and cid < len(self.names) else str(cid)

    def run(self, max_actions: int = 200) -> "TurnPlay":
        b = self.b
        S = Slots(b)
        S.observe()
        self.gold_start = S.describe(0)[0]
        turn = b.slot_turn(0)
        # A turn with nothing to decide (the seat is a real board, or the game
        # is over) has no actions.
        for _ in range(max_actions):
            S.observe()
            if S.state[0] == 2 or b.slot_turn(0) != turn:
                # The turn is over: the game has moved to the next one, whose
                # shop is not this turn's to play.
                break
            # A choice waiting (a target, a Discover, a Choose One) is answered
            # by the search too: decide_batch hands those to the network.
            desc = S.describe(0)
            a = self.searcher.decide_batch(b, key=("turnplay", id(self)))
            kind = int(KIND_OF[a])
            if kind == K_BUY and a < len(desc[8]):
                self.bought.append(desc[8][a][0])
            elif kind == K_SELL and a - FIRST["sell"] < len(desc[9]):
                self.sold.append(desc[9][a - FIRST["sell"]][0])
            elif kind == K_PLAY:
                h, _ = play_parts(a)
                if h < len(desc[7]):
                    self.played.append(desc[7][h][0])
            b.act(0, [a])
            self.actions.append(a)
            if len(S.lines) > 1:
                S.truncate(1)
        S.observe()
        self.gold_end = S.describe(0)[0]
        return self

    def cycles(self) -> list[int]:
        """The cards bought or played and then sold this turn."""
        return [c for c in set(self.bought) | set(self.played) if c in self.sold]

    def summary(self) -> dict:
        return {
            "actions": len(self.actions),
            "bought": [self.name(c) for c in self.bought],
            "played": [self.name(c) for c in self.played],
            "sold": [self.name(c) for c in self.sold],
            "cycles": [self.name(c) for c in self.cycles()],
            "gold": (self.gold_start, self.gold_end),
        }
