"""Tests for the training wheels' fade and packed bonus storage.

    cd train && ../.venv/bin/python -m unittest test_wheels
"""

import unittest

import numpy as np
import torch

import bgpy
import ppo


class WheelsFadeTest(unittest.TestCase):
    def test_no_fade_when_both_zero(self):
        self.assertEqual(ppo.wheels_scale(0, 0, 0), 1.0)
        self.assertEqual(ppo.wheels_scale(10_000_000, 0, 0), 1.0)

    def test_full_then_linear_then_none(self):
        points = {0: 1.0, 100: 1.0, 150: 0.75, 200: 0.5, 250: 0.25, 300: 0.0, 400: 0.0}
        for games, want in points.items():
            self.assertAlmostEqual(ppo.wheels_scale(games, 100, 300), want, places=12)

    def test_starting_at_zero(self):
        self.assertAlmostEqual(ppo.wheels_scale(0, 0, 100), 1.0)
        self.assertAlmostEqual(ppo.wheels_scale(50, 0, 100), 0.5)
        self.assertAlmostEqual(ppo.wheels_scale(100, 0, 100), 0.0)

    def test_instant_off(self):
        self.assertEqual(ppo.wheels_scale(99, 100, 100), 1.0)
        self.assertEqual(ppo.wheels_scale(100, 100, 100), 0.0)
        self.assertEqual(ppo.wheels_scale(101, 100, 100), 0.0)


class CompInputTest(unittest.TestCase):
    def test_none_and_one_hot(self):
        self.assertIsNone(ppo.comp_onehot(np.array([0, 3]), 0, "cpu"))
        out = ppo.comp_onehot(np.array([-1, 3]), 5, "cpu")
        self.assertEqual(tuple(out.shape), (2, 5))
        self.assertEqual(out[0].sum().item(), 0.0)
        self.assertEqual(out[1, 3].item(), 1.0)
        self.assertEqual(out[1].sum().item(), 1.0)


class BonusStoreTest(unittest.TestCase):
    def test_round_trip(self):
        rows = 5
        store = ppo.BonusStore(rows)
        for scale, b, hp in ((1.0, 8.0, -10.0), (0.37, 8.0, -10.0), (0.5, 0.0, -10.0)):
            store.set_scale(scale, b, hp)
            raw = np.zeros((rows, bgpy.N_ACTIONS), np.float32)
            raw[0, 1] = np.float32(b)
            raw[0, 2] = np.float32(b) / np.float32(2)
            raw[0, 3] = np.float32(hp)
            raw[0, 4] = np.float32(b) + np.float32(hp)
            raw[0, 5] = -np.inf
            raw[1, 6] = np.float32(hp) + np.float32(b) / np.float32(2)
            store.put(np.arange(rows), raw)
            got = store.get(np.ones(rows, bool), "cpu").numpy()
            want = raw * np.float32(scale)
            np.testing.assert_array_equal(got, want)

    def test_zero_scale_is_all_zero(self):
        store = ppo.BonusStore(2)
        store.set_scale(0.0, 8.0, -10.0)
        raw = np.zeros((2, bgpy.N_ACTIONS), np.float32)
        raw[0, 0] = 8.0
        raw[0, 1] = -np.inf
        store.put(np.arange(2), raw)
        got = store.get(np.ones(2, bool), "cpu")
        self.assertTrue(torch.equal(got, torch.zeros_like(got)))

    def test_selection(self):
        store = ppo.BonusStore(3)
        store.set_scale(1.0, 4.0, -1.0)
        raw = np.zeros((3, bgpy.N_ACTIONS), np.float32)
        raw[0, 0] = 4.0
        raw[2, 7] = 4.0
        store.put(np.arange(3), raw)
        got = store.get(np.array([False, False, True]), "cpu").numpy()
        self.assertEqual(got.shape, (1, bgpy.N_ACTIONS))
        self.assertEqual(got[0, 7], np.float32(4.0))
        self.assertEqual(got[0, 0], 0.0)


if __name__ == "__main__":
    unittest.main()
