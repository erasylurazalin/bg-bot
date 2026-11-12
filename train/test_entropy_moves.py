"""Tests for counting all Move actions as one entropy atom.

    cd train && ../.venv/bin/python -m unittest test_entropy_moves
"""

import math
import unittest

import torch

import ppo


class EntropyMovesTest(unittest.TestCase):
    def test_one_move_atom_matches_the_hand_computed_value(self):
        probs = torch.tensor([[0.1, 0.2, 0.3, 0.1, 0.3]], dtype=torch.float64)
        move_idx = torch.tensor([1, 3])
        dist = torch.distributions.Categorical(probs=probs)
        # The moves sum to 0.3: three ordinary actions at 0.3 (two of them
        # non-moves) and one at 0.1, plus the 0.3 move atom.
        want = -(0.1 * math.log(0.1) + 3 * 0.3 * math.log(0.3))
        got = ppo.entropy_term(dist, True, move_idx)
        self.assertAlmostEqual(got.item(), want, places=12)

    def test_no_legal_move_equals_the_plain_entropy(self):
        probs = torch.tensor([[0.2, 0.0, 0.4, 0.0, 0.4]], dtype=torch.float64)
        move_idx = torch.tensor([1, 3])
        dist = torch.distributions.Categorical(probs=probs)
        got = ppo.entropy_term(dist, True, move_idx)
        self.assertAlmostEqual(got.item(), dist.entropy().mean().item(), places=12)

    def test_flag_off_returns_the_plain_entropy(self):
        probs = torch.tensor([[0.1, 0.2, 0.3, 0.1, 0.3]], dtype=torch.float64)
        move_idx = torch.tensor([1, 3])
        dist = torch.distributions.Categorical(probs=probs)
        self.assertEqual(ppo.entropy_term(dist, False, move_idx).item(),
                         dist.entropy().mean().item())


if __name__ == "__main__":
    unittest.main()
