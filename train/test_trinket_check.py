"""Tests for the forced trinket pick (CPU only).

    cd train && ../.venv/bin/python -m unittest test_trinket_check

The three flags' choices are pure functions in `train/fixed_eval.py`; the
`own` check runs two tiny fixed-eval game sets and compares the rows.
"""

import random
import sys
import unittest
from collections import Counter
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import numpy as np
import torch

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE / "evaltools"))

import bgpy
import fixed_eval as fe
import real_boards

BASE = ROOT / "train/runs/r1115-school-text/best.pt"


def parsed(extra=()):
    argv = ["fixed_eval.py", str(BASE), "--games", "2", "--cpu", *extra]
    with mock.patch.object(sys, "argv", argv):
        args = fe.parse_args()
    args.learner_heroes = []
    args.era_folder = None
    args.era_turn_min = args.era_turn_max = 0
    args.era_split = None
    return args


class ForcedTrinketTest(unittest.TestCase):
    def test_popular_takes_the_most_streamer_picked_and_breaks_ties_by_slot(self):
        names = ["A", "B", "C", "D"]
        ids = [0, 1, 2, 3]
        counts = Counter({"B": 7, "C": 7, "A": 1})
        # B and C tie on 7; B is at the lower offer position.
        self.assertEqual(fe.popular_trinket_slot(ids, names, counts), 1)
        # Everything at 0 is a tie, so the first offer position wins.
        self.assertEqual(fe.popular_trinket_slot(ids, names, Counter()), 0)
        # A clear winner outside the first slot still wins.
        self.assertEqual(fe.popular_trinket_slot(ids, names, Counter({"D": 9})), 3)
        # The pick is an offer position, and apply_trinket_pick takes that
        # position's action, not the trinket id.
        b = SimpleNamespace(
            mask=np.zeros((1, bgpy.N_ACTIONS), bool),
            obs=np.zeros((1, bgpy.OBS_SIZE), np.float32),
        )
        for slot, i in enumerate(ids):
            b.mask[0, fe.TRINKET_BASE + slot] = True
            at = bgpy.SCALAR_TRINKET_OPTION + slot * bgpy.TRINKET_OPTION_FEATURES
            b.obs[0, at] = i + 1
            b.obs[0, at + 1] = i
        acts = np.array([fe.TRINKET_BASE + 0])
        fe.apply_trinket_pick(acts, b, "popular", names, counts, {}, 0)
        self.assertEqual(int(acts[0]), fe.TRINKET_BASE + 1)

    def test_random_is_the_same_for_the_same_seed_and_differs_across_seeds(self):
        ids = [0, 1, 2, 3]
        rng_a = random.Random(123)
        same_a = [fe.forced_trinket_slot("random", ids, rng=rng_a)
                  for _ in range(24)]
        rng_b = random.Random(123)
        same_b = [fe.forced_trinket_slot("random", ids, rng=rng_b)
                  for _ in range(24)]
        rng_c = random.Random(124)
        other = [fe.forced_trinket_slot("random", ids, rng=rng_c)
                 for _ in range(24)]
        self.assertEqual(same_a, same_b)
        self.assertNotEqual(same_a, other)
        # Only the offered positions are possible, uniformly over the two.
        rng_two = random.Random(999)
        two = [fe.forced_trinket_slot("random", [0, 2], rng=rng_two)
               for _ in range(40)]
        self.assertEqual(set(two), {0, 1})

    def test_own_leaves_the_eval_output_unchanged(self):
        args_no = parsed()
        del args_no.trinket_pick  # the namespace as it was before the flag
        args_own = parsed(["--trinket-pick", "own"])
        self.assertEqual(args_own.trinket_pick, "own")
        real, _ = real_boards.load(fe.FROZEN)
        dev = torch.device("cpu")
        info = fe.load_ckpt(str(BASE), dev)
        rows_no = fe.eval_checkpoint(info, args_no, real, dev)
        rows_own = fe.eval_checkpoint(info, args_own, real, dev)
        self.assertEqual(rows_no, rows_own)


if __name__ == "__main__":
    unittest.main()
