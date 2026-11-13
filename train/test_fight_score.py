"""Tests for the graded fight score (--fight-score).

    cd train && ../.venv/bin/python -m unittest test_fight_score
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


def short_rollout(score_weight=0.0, score_from=1, steps=200, n=1, seed=17,
                  record_fights=False, real=None, use_score=True):
    """A short rollout of one learner against seven real seats, with the
    score's crediting path (and placement rewards), returning the reward array,
    the resolved fight events and the environment.

    `use_score=False` is today's rollout: no fight-score code runs. With
    `use_score=True` and `score_weight` 0 the code path runs; the reward must
    not change. Every event carries the engine's uncapped numbers read at the
    moment the term was credited.
    """
    if real is None:
        real, _ = real_boards.load()
        if not real:
            raise unittest.SkipTest("no real boards in boards/real")
    env = bgpy.VecEnv(n, seed, [1, 0, 0, 0, 0, 7], real_boards=real,
                      real_lobby_filter=True)
    if record_fights:
        env.record_fights(True)
    obs = np.zeros((n, bgpy.OBS_SIZE), np.float32)
    mask = np.zeros((n, bgpy.N_ACTIONS), bool)
    seat = np.zeros(n, np.int64)
    kind = np.zeros(n, np.int64)
    places = np.zeros((n, P), np.int8)
    tier_turns = np.zeros((n, P, bgpy.MAX_TIER), np.int8)
    tribes = np.zeros((n, P, bgpy.TRIBES), np.int8)
    reset = np.zeros(n, bool)
    kinds = np.zeros((n, P), np.int64)
    dealt = np.zeros((n, P), np.int32)
    taken = np.zeros((n, P), np.int32)
    rews = np.zeros((steps, n), np.float64)
    last = np.full((n, P), -1, np.int64)
    pending = np.full((n, P), -1, np.int64)
    pending_turn = np.full((n, P), -1, np.int64)
    events = []
    fight_turns = 0
    env.kinds(kinds)

    for step in range(steps):
        env.observe(obs, mask, seat, kind)
        if use_score:
            env.fight_damage(dealt, taken)
            # A learner's next view resolves the fight after its pending turn.
            for e in np.nonzero(kind == bgpy.LEARNER)[0]:
                e, s = int(e), int(seat[e])
                if pending[e, s] < 0 or obs[e, bgpy.SCALAR_TURN] <= pending_turn[e, s]:
                    continue
                pend_step = int(pending[e, s])
                before = rews[pend_step, e]
                term = ppo.credit_fight_score(
                    rews, pending, pending_turn, e, s, dealt[e, s], taken[e, s],
                    score_weight, score_from)
                events.append(dict(step=step, rew_step=pend_step, env=e, seat=s, at="view",
                                   turn=int(pending_turn[e, s]), dealt=int(dealt[e, s]),
                                   taken=int(taken[e, s]), term=term,
                                   change=float(rews[pend_step, e] - before),
                                   snapshot=env.fight_snapshot(e, s) if record_fights else None))
        acts = np.array([pick_action(mask[e]) for e in range(n)], np.int64)
        ended = acts == bgpy.END_TURN
        le = np.nonzero(kind == bgpy.LEARNER)[0]
        before_kinds = kinds.copy()
        for e in le:
            e, s = int(e), int(seat[e])
            last[e, s] = step
            if ended[e]:
                if use_score:
                    pending[e, s] = step
                    pending_turn[e, s] = int(obs[e, bgpy.SCALAR_TURN])
                    fight_turns += 1
        env.step(acts, places, tier_turns, tribes, reset)
        if use_score and places.any():
            env.fight_damage(dealt, taken)
        for e, s in zip(*np.nonzero(places)):
            e, s = int(e), int(s)
            if before_kinds[e, s] != bgpy.LEARNER or last[e, s] < 0:
                continue
            rews[last[e, s], e] += ppo.reward(int(places[e, s]))
            if not use_score:
                continue
            before = rews[last[e, s], e]
            term = ppo.credit_fight_score(
                rews, pending, pending_turn, e, s, dealt[e, s], taken[e, s],
                score_weight, score_from)
            events.append(dict(step=step, rew_step=int(last[e, s]), env=e, seat=s, at="place",
                               place=int(places[e, s]),
                               turn=int(pending_turn[e, s]), dealt=int(dealt[e, s]),
                               taken=int(taken[e, s]), term=term,
                               change=float(rews[last[e, s], e] - before),
                               snapshot=env.fight_snapshot(e, s) if record_fights else None))
        if reset.any():
            for e in np.nonzero(reset)[0]:
                last[e] = -1
            env.kinds(kinds)

    return rews, events, env, fight_turns


class FightScoreTest(unittest.TestCase):
    def test_zero_weight_is_exactly_todays_rewards(self):
        # Today's rollout is the same mini rollout with no fight-score code.
        real, _ = real_boards.load()
        base, _, _, _ = short_rollout(steps=80, n=2, real=real, use_score=False)
        zero, _, _, _ = short_rollout(score_weight=0.0, steps=80, n=2, real=real,
                                      use_score=True)
        np.testing.assert_array_equal(base, zero)
        self.assertTrue(base.any(), "the rollout paid a placement reward")

    def test_no_score_before_from_turn(self):
        _, events, _, _ = short_rollout(score_weight=0.5, score_from=5, steps=90)
        early = [ev for ev in events if ev["term"] is not None and ev["turn"] < 5]
        self.assertTrue(early, "the rollout had turns before 5")
        self.assertTrue(any(ev["dealt"] or ev["taken"] for ev in early),
                        "an early fight had damage")
        for ev in early:
            self.assertEqual(ev["term"], 0.0)
            self.assertEqual(ev["change"], 0.0)

    def test_score_equals_weight_times_uncapped_over_48(self):
        weight = 0.5
        _, events, _, _ = short_rollout(score_weight=weight, score_from=1, steps=90)
        paid = [ev for ev in events if ev["term"] is not None]
        self.assertTrue(paid, "the rollout had no fights")
        for ev in paid:
            want = weight * (ev["dealt"] - ev["taken"]) / ppo.FIGHT_SCORE_SCALE
            self.assertAlmostEqual(ev["term"], want, places=9)
            self.assertAlmostEqual(ev["change"], want, places=9)
        self.assertTrue(any(ev["dealt"] for ev in paid), "a won fight was seen")
        self.assertTrue(any(ev["taken"] for ev in paid), "a lost fight was seen")

    def test_fade_falls_to_zero_at_the_planned_end(self):
        f = ppo.fight_score_fade_scale
        self.assertEqual(f(10, 0, 100, 0.0), 1.0)
        self.assertEqual(f(74, 0, 100, 0.25), 1.0)
        self.assertAlmostEqual(f(87.5, 0, 100, 0.25), 0.5)
        self.assertEqual(f(100, 0, 100, 0.25), 0.0)
        self.assertEqual(f(120, 0, 100, 0.25), 0.0)
        # A resumed run fades over its own remaining minutes (50 to 110).
        self.assertAlmostEqual(f(80, 50, 110, 0.5), 1.0)
        self.assertAlmostEqual(f(95, 50, 110, 0.5), 0.5)
        self.assertEqual(f(110, 50, 110, 0.5), 0.0)

    def test_a_knockout_counts_its_uncapped_damage_as_taken(self):
        weight = 0.5
        _, events, _, _ = short_rollout(score_weight=weight, score_from=1, steps=200)
        knockouts = [ev for ev in events
                     if ev["at"] == "place" and ev["place"] != 1 and ev["taken"] > 0
                     and ev["term"] is not None]
        self.assertTrue(knockouts, "the rollout had a knockout with damage taken")
        for ev in knockouts:
            want = -weight * ev["taken"] / ppo.FIGHT_SCORE_SCALE
            self.assertAlmostEqual(ev["term"], want, places=9)
            self.assertEqual(ev["dealt"], 0)

    def test_fight_export_agrees_with_the_recorded_uncapped_damage(self):
        # Re-fight the exported pair of a learner win and compare the export's
        # uncapped number (tier plus surviving minion tiers) with the engine's
        # recorded one. Only deterministic wins (same raw on every re-run) are
        # compared; the Rust test holds the capped-versus-uncapped case.
        _, events, _, _ = short_rollout(score_weight=0.5, score_from=1, steps=200,
                                        record_fights=True)
        checked = 0
        for ev in events:
            if ev["snapshot"] is None or ev["dealt"] <= 0 or ev["taken"] != 0:
                continue
            me, opp, opp_seat = ev["snapshot"]
            if opp_seat is None:  # a ghost win deals no damage and is not this case
                continue
            fight = bgpy.BoardFight(me, opp)
            if not fight.complete():
                continue
            raw = set()
            capped = set()
            for seed in range(16):
                info = fight.fight(1, seed)
                if info["wins"]:
                    raw.add(info["damage_dealt_uncapped"])
                    capped.add(info["damage_dealt"])
            if len(raw) != 1:
                continue  # surviving tiers varied between re-runs
            self.assertEqual(raw.pop(), ev["dealt"])
            self.assertTrue(all(c <= ev["dealt"] for c in capped))
            checked += 1
        self.assertGreater(checked, 0, "no win was cross-checked")


if __name__ == "__main__":
    unittest.main()
