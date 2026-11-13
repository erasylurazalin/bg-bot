"""Tests for the never-negative win score (--win-score).

    cd train && ../.venv/bin/python -m unittest test_win_score
"""

import unittest

import numpy as np

import bgpy
import ppo
import real_boards

P = bgpy.PLAYERS
K = bgpy.ACTION_KINDS
BASES = np.cumsum([0] + [c for _, c in K])


def pick_action(mask):
    """A deterministic learner action: play, buy, buy a spell, level, roll,
    else End Turn. When the game is waiting on a Discover or a target, only
    the legal actions are offered, so take the first one."""
    if not mask[bgpy.END_TURN]:
        return int(np.nonzero(mask)[0][0])
    for want in ("play", "buy", "buy_spell", "level", "roll"):
        kind = next(i for i, (name, _) in enumerate(K) if name == want)
        hits = np.nonzero(mask[BASES[kind]:BASES[kind + 1]])[0]
        if len(hits):
            return int(BASES[kind] + hits[0])
    return bgpy.END_TURN


def short_rollout(win_weight=0.0, win_from=9, steps=200, n=1, seed=17,
                  real=None, use_win=True):
    """A short rollout of one learner against seven real seats, with the win
    score's crediting path (and placement rewards), returning the reward array
    and the resolved fight events.

    `use_win=False` is today's rollout: no win-score code runs. With
    `use_win=True` and `win_weight` 0 the code path runs; the reward must not
    change. Every event carries the outcome the engine's won/lost scalars gave
    at the moment the term was credited.
    """
    if real is None:
        real, _ = real_boards.load()
        if not real:
            raise unittest.SkipTest("no real boards in boards/real")
    env = bgpy.VecEnv(n, seed, [1, 0, 0, 0, 0, 7], real_boards=real,
                      real_lobby_filter=True)
    obs = np.zeros((n, bgpy.OBS_SIZE), np.float32)
    mask = np.zeros((n, bgpy.N_ACTIONS), bool)
    seat = np.zeros(n, np.int64)
    kind = np.zeros(n, np.int64)
    places = np.zeros((n, P), np.int8)
    tier_turns = np.zeros((n, P, bgpy.MAX_TIER), np.int8)
    tribes = np.zeros((n, P, bgpy.TRIBES), np.int8)
    reset = np.zeros(n, bool)
    kinds = np.zeros((n, P), np.int64)
    rews = np.zeros((steps, n), np.float64)
    last = np.full((n, P), -1, np.int64)
    pending = np.full((n, P), -1, np.int64)
    pending_turn = np.full((n, P), -1, np.int64)
    events = []
    env.kinds(kinds)

    for step in range(steps):
        env.observe(obs, mask, seat, kind)
        if use_win:
            # A learner's next view resolves the fight after its pending turn.
            for e in np.nonzero(kind == bgpy.LEARNER)[0]:
                e, s = int(e), int(seat[e])
                if pending[e, s] < 0 or obs[e, bgpy.SCALAR_TURN] <= pending_turn[e, s]:
                    continue
                pend_step = int(pending[e, s])
                before = rews[pend_step, e]
                outcome = ppo.win_score_outcome(
                    obs[e, bgpy.SCALAR_WON_LAST_COMBAT],
                    obs[e, bgpy.SCALAR_LOST_LAST_COMBAT])
                term = ppo.credit_win_score(
                    rews, pending, pending_turn, e, s, outcome, win_weight,
                    win_from)
                events.append(dict(step=step, rew_step=pend_step, env=e, seat=s,
                                   at="view", turn=int(pending_turn[e, s]),
                                   outcome=outcome, term=term,
                                   change=float(rews[pend_step, e] - before)))
        acts = np.array([pick_action(mask[e]) for e in range(n)], np.int64)
        # The same turn end the main loop uses: End Turn, or the last action.
        ended = (acts == bgpy.END_TURN) | (obs[:, bgpy.SCALAR_ACTIONS_LEFT] <= 1)
        le = np.nonzero(kind == bgpy.LEARNER)[0]
        before_kinds = kinds.copy()
        for e in le:
            e, s = int(e), int(seat[e])
            last[e, s] = step
            if ended[e] and use_win:
                pending[e, s] = step
                pending_turn[e, s] = int(obs[e, bgpy.SCALAR_TURN])
        env.step(acts, places, tier_turns, tribes, reset)
        for e, s in zip(*np.nonzero(places)):
            e, s = int(e), int(s)
            if before_kinds[e, s] != bgpy.LEARNER or last[e, s] < 0:
                continue
            rews[last[e, s], e] += ppo.reward(int(places[e, s]))
            if not use_win:
                continue
            before = rews[last[e, s], e]
            outcome = ppo.place_win_score_outcome(int(places[e, s]))
            term = ppo.credit_win_score(
                rews, pending, pending_turn, e, s, outcome, win_weight,
                win_from)
            events.append(dict(step=step, rew_step=int(last[e, s]), env=e,
                               seat=s, at="place", place=int(places[e, s]),
                               turn=int(pending_turn[e, s]), outcome=outcome,
                               term=term,
                               change=float(rews[last[e, s], e] - before)))
        if reset.any():
            for e in np.nonzero(reset)[0]:
                last[e] = -1
            env.kinds(kinds)

    return rews, events


class WinScoreTest(unittest.TestCase):
    def test_zero_weight_is_exactly_todays_rewards(self):
        real, _ = real_boards.load()
        base, _ = short_rollout(steps=80, n=2, real=real, use_win=False)
        zero, _ = short_rollout(win_weight=0.0, steps=80, n=2, real=real,
                                use_win=True)
        np.testing.assert_array_equal(base, zero)
        self.assertTrue(base.any(), "the rollout paid a placement reward")

    def test_no_score_before_from_turn(self):
        _, events = short_rollout(win_weight=0.5, win_from=5, steps=90)
        early = [ev for ev in events if ev["term"] is not None and ev["turn"] < 5]
        self.assertTrue(early, "the rollout had turns before 5")
        for ev in early:
            self.assertEqual(ev["term"], 0.0)
            self.assertEqual(ev["change"], 0.0)
            self.assertIn(ev["outcome"], (0.0, 0.5, 1.0))

    def test_win_tie_loss_and_from_turn(self):
        weight = 0.3
        steps, envs, players = 8, 2, P
        rews = np.zeros((steps, envs))
        pending = np.full((envs, players), -1, np.int64)
        pending_turn = np.full((envs, players), -1, np.int64)

        def end(env, seat, step, turn):
            pending[env, seat] = step
            pending_turn[env, seat] = turn

        def resolve(env, seat, outcome):
            return ppo.credit_win_score(
                rews, pending, pending_turn, env, seat, outcome, weight, 9)

        # Before turn 9 pays 0, and still resolves the pending turn.
        end(0, 1, 0, 8)
        self.assertEqual(resolve(0, 1, ppo.win_score_outcome(1.0, 0.0)), 0.0)
        self.assertEqual(rews[0, 0], 0.0)
        self.assertEqual(pending[0, 1], -1)

        # A win pays W, credited to the turn-end step, not the later step.
        end(0, 1, 1, 9)
        self.assertAlmostEqual(resolve(0, 1, ppo.win_score_outcome(1.0, 0.0)),
                               weight)
        self.assertAlmostEqual(rews[1, 0], weight, places=6)
        self.assertEqual(rews[4, 0], 0.0)

        # A tie pays W/2.
        end(0, 1, 2, 10)
        self.assertAlmostEqual(resolve(0, 1, ppo.win_score_outcome(0.0, 0.0)),
                               weight / 2)
        self.assertAlmostEqual(rews[2, 0], weight / 2, places=6)

        # A loss pays 0 and still resolves the pending turn.
        end(0, 1, 3, 11)
        self.assertEqual(resolve(0, 1, ppo.win_score_outcome(0.0, 1.0)), 0.0)
        self.assertEqual(rews[3, 0], 0.0)
        self.assertEqual(pending[0, 1], -1)

        # A knockout pays 0, whatever the place; 1st place is a win.
        end(1, 2, 4, 9)
        self.assertEqual(resolve(1, 2, ppo.place_win_score_outcome(5)), 0.0)
        self.assertEqual(rews[4, 1], 0.0)
        end(1, 4, 5, 9)
        self.assertAlmostEqual(resolve(1, 4, ppo.place_win_score_outcome(1)),
                               weight)
        self.assertAlmostEqual(rews[5, 1], weight, places=6)

        # No pending turn credits nothing.
        self.assertIsNone(resolve(0, 0, 1.0))
        self.assertAlmostEqual(rews.sum(), weight + weight / 2 + weight)

    def test_a_knockout_pays_zero(self):
        weight = 0.5
        _, events = short_rollout(win_weight=weight, win_from=1, steps=200)
        knockouts = [ev for ev in events
                     if ev["at"] == "place" and ev["place"] != 1
                     and ev["term"] is not None]
        self.assertTrue(knockouts, "the rollout had a knockout")
        for ev in knockouts:
            self.assertEqual(ev["term"], 0.0)
            self.assertEqual(ev["change"], 0.0)
        self.assertEqual(ppo.place_win_score_outcome(1), 1.0)
        for place in (2, 3, 4, 5, 6, 7, 8):
            self.assertEqual(ppo.place_win_score_outcome(place), 0.0)

    def test_fade_falls_to_zero_at_the_planned_end(self):
        f = ppo.win_score_fade_scale
        self.assertEqual(f(10, 0, 100, 0.0), 1.0)
        self.assertEqual(f(74, 0, 100, 0.25), 1.0)
        self.assertAlmostEqual(f(87.5, 0, 100, 0.25), 0.5)
        self.assertEqual(f(100, 0, 100, 0.25), 0.0)
        self.assertEqual(f(120, 0, 100, 0.25), 0.0)
        # A resumed run fades over its own remaining minutes (50 to 110).
        self.assertAlmostEqual(f(80, 50, 110, 0.5), 1.0)
        self.assertAlmostEqual(f(95, 50, 110, 0.5), 0.5)
        self.assertEqual(f(110, 50, 110, 0.5), 0.0)


if __name__ == "__main__":
    unittest.main()
