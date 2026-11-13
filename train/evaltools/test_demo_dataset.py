"""Tests for `train/evaltools/demo_dataset.py`.

    cd train/evaltools && ../../.venv/bin/python -m unittest test_demo_dataset

A few turns are replayed twice. The saved observation, legal mask and action
must be exactly what a fresh `EraTurnPath` shows at the same step, and every
saved action must be legal in its own mask. The split is checked to be fixed,
disjoint and the requested size.
"""

import unittest
from pathlib import Path
import sys

import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "train"))
sys.path.insert(0, str(HERE))

import bgpy
import demo_dataset as dd


class DemoDatasetTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        replays, bad = bgpy.era_replays(
            str(dd.ACTIONS), str(dd.ROOT / "ref/bg-cards.json")
        )
        if bad:
            raise unittest.SkipTest(f"the replay loader reported: {bad[:2]}")
        cls.replay = replays[0]
        cls.turn = None
        cls.rows = None
        for turn in cls.replay.turns(1):
            rows, _skipped, error, _logged = dd.replay_turn(cls.replay, turn)
            if error is None and len(rows) >= 2:
                cls.turn = turn
                cls.rows = rows
                break
        if cls.rows is None:
            raise unittest.SkipTest("no turn of the first game replays")

    def path_at(self, step):
        path = self.replay.path(self.turn, dd.REPLAY_SEED, True)
        while path.steps_on_path < step:
            action = path.era_action()
            self.assertGreaterEqual(action, 0, "the logged steps ended early")
            self.assertTrue(path.act(action), "the path left Era's steps")
        self.assertEqual(path.steps_on_path, step)
        obs = np.zeros(bgpy.OBS_SIZE, dtype=np.float32)
        mask = np.zeros(bgpy.N_ACTIONS, dtype=bool)
        path.observe(obs, mask)
        return path, obs, mask

    def test_saved_observation_and_mask_match_a_fresh_path(self):
        for row in self.rows[:3] + [self.rows[-1]]:
            path, obs, mask = self.path_at(row["step"])
            np.testing.assert_array_equal(obs, row["obs"])
            np.testing.assert_array_equal(mask, row["mask"])
            self.assertEqual(path.era_action(), row["action"])

    def test_saved_actions_are_legal(self):
        for row in self.rows:
            self.assertTrue(
                row["mask"][row["action"]],
                f"step {row['step']} action {row['action']} is illegal",
            )

    def test_split_is_fixed_and_disjoint(self):
        files = dd.action_files()
        first, _ = dd.split_games(files, dd.SPLIT_SEED, dd.HOLDOUT)
        second, _ = dd.split_games(files, dd.SPLIT_SEED, dd.HOLDOUT)
        self.assertEqual(first, second)
        held = {name for name, split in first.items() if split == "heldout"}
        train = {name for name, split in first.items() if split == "train"}
        self.assertEqual(held | train, set(files))
        self.assertFalse(held & train)
        self.assertEqual(len(held), 14)


if __name__ == "__main__":
    unittest.main()
