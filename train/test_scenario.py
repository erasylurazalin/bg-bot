"""Tests for scenario starts (docs/briefs/comp-starts.md).

    cd train && ../.venv/bin/python -m unittest test_scenario
"""

import unittest
from pathlib import Path

import numpy as np

import bgpy
import comp_specs
import ppo
import real_boards

ROOT = Path(__file__).resolve().parent.parent
COMPS = ROOT / "data" / "comps" / "comps.toml"
P = bgpy.PLAYERS


def boards():
    real, unknown = real_boards.load()
    if not real:
        raise unittest.SkipTest("no real boards in boards/real")
    return real, unknown


def comp_index(name):
    return next(i for i, s in enumerate(comp_specs.load_comps(str(COMPS))) if s["name"] == name)


class ScenarioOffTest(unittest.TestCase):
    def test_share_zero_is_the_plain_game(self):
        real, _ = boards()
        names = ["Beasts - Beetles"]
        a = bgpy.VecEnv(4, 1234, [1, 0, 0, 0, 0, 7], real_boards=real)
        b = bgpy.VecEnv(4, 1234, [1, 0, 0, 0, 0, 7], real_boards=real,
                        scenario_comps=str(COMPS), scenario_names=names,
                        scenario_share=0.0, scenario_turn_min=7, scenario_turn_max=11)
        n = 4
        oa = np.zeros((n, bgpy.OBS_SIZE), np.float32)
        ob = np.zeros((n, bgpy.OBS_SIZE), np.float32)
        ma = np.zeros((n, bgpy.N_ACTIONS), bool)
        mb = np.zeros((n, bgpy.N_ACTIONS), bool)
        sa, sb = np.zeros(n, np.int64), np.zeros(n, np.int64)
        ka, kb = np.zeros(n, np.int64), np.zeros(n, np.int64)
        pa, pb = np.zeros((n, P), np.int8), np.zeros((n, P), np.int8)
        ta, tb = np.zeros((n, P, bgpy.MAX_TIER), np.int8), np.zeros((n, P, bgpy.MAX_TIER), np.int8)
        ra, rb = np.zeros((n, P, bgpy.TRIBES), np.int8), np.zeros((n, P, bgpy.TRIBES), np.int8)
        da, db = np.zeros(n, bool), np.zeros(n, bool)
        for _ in range(120):
            a.observe(oa, ma, sa, ka)
            b.observe(ob, mb, sb, kb)
            np.testing.assert_array_equal(oa, ob)
            np.testing.assert_array_equal(ma, mb)
            np.testing.assert_array_equal(sa, sb)
            np.testing.assert_array_equal(ka, kb)
            acts = ma.argmax(1).astype(np.int64)  # the first legal action
            a.step(acts, pa, ta, ra, da)
            b.step(acts, pb, tb, rb, db)
            np.testing.assert_array_equal(pa, pb)
            np.testing.assert_array_equal(ta, tb)
            np.testing.assert_array_equal(ra, rb)
            np.testing.assert_array_equal(da, db)


class ScenarioStartTest(unittest.TestCase):
    def test_turn_body_and_own_comp(self):
        real, _ = boards()
        names = ["Beasts - Tasty Lobstah", "Elementals - Unbound Tempest", "Beasts - Beetles"]
        env = bgpy.VecEnv(3, 77, [1, 0, 0, 0, 0, 7], real_boards=real,
                          scenario_comps=str(COMPS), scenario_names=names,
                          scenario_share=1.0, scenario_turn_min=9, scenario_turn_max=9)
        env.set_overlay(str(COMPS), 0.0)
        n = 3
        obs = np.zeros((n, bgpy.OBS_SIZE), np.float32)
        mask = np.zeros((n, bgpy.N_ACTIONS), bool)
        seat = np.zeros(n, np.int64)
        kind = np.zeros(n, np.int64)
        forced = np.full(n, -1, np.int64)
        flags = np.zeros((n, P, bgpy.OVERLAY_FLAGS), np.int32)
        env.observe(obs, mask, seat, kind)
        env.overlay_probe(forced, flags)
        bodies = np.asarray(env.scenario_bodies())
        want = [comp_index(x) for x in names]
        self.assertTrue((obs[:, bgpy.SCALAR_TURN] == 9).all())
        for i in range(n):
            self.assertEqual(int(kind[i]), bgpy.LEARNER)
            self.assertTrue((bodies[i, seat[i]] >= 0).any(), "a scenario body is on the board")
            ci = int(flags[i, seat[i], ppo.F_COMP]) - 1
            self.assertIn(ci, want)
            self.assertGreaterEqual(int(obs[i, bgpy.SCALAR_TIER]), 4)

    def test_own_comp_beats_the_file_order(self):
        real, _ = boards()
        beetles = comp_index("Beasts - Beetles")
        tasty = comp_index("Beasts - Tasty Lobstah")
        self.assertLess(tasty, beetles, "Tasty is earlier in the file")
        env = bgpy.VecEnv(1, 88, [1, 0, 0, 0, 0, 7], real_boards=real,
                          scenario_comps=str(COMPS), scenario_names=["Beasts - Beetles"],
                          scenario_share=1.0, scenario_turn_min=7, scenario_turn_max=7)
        env.set_overlay(str(COMPS), 0.0)
        obs = np.zeros((1, bgpy.OBS_SIZE), np.float32)
        mask = np.zeros((1, bgpy.N_ACTIONS), bool)
        seat = np.zeros(1, np.int64)
        kind = np.zeros(1, np.int64)
        forced = np.full(1, -1, np.int64)
        flags = np.zeros((1, P, bgpy.OVERLAY_FLAGS), np.int32)
        env.observe(obs, mask, seat, kind)
        env.overlay_probe(forced, flags)
        self.assertEqual(int(flags[0, seat[0], ppo.F_COMP]) - 1, beetles)


if __name__ == "__main__":
    unittest.main()
