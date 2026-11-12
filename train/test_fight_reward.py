"""Tests for the per-turn fight reward's crediting logic.

    cd train && ../.venv/bin/python -m unittest test_fight_reward
"""

import unittest

import numpy as np
import torch

import ppo


class FightRewardTest(unittest.TestCase):
    def test_hand_made_sequence(self):
        weight = 0.3
        steps, envs, players = 7, 2, 8
        rews = torch.zeros((steps, envs))
        pending = np.full((envs, players), -1, np.int64)
        turns = 0

        def end_turn(env, seat, step):
            nonlocal turns
            pending[env, seat] = step
            turns += 1

        def resolve(env, seat, outcome):
            return ppo.credit_fight(rews, pending, env, seat, outcome, weight)

        # A win, credited to the turn-end step, not to the later step where
        # the result is seen.
        end_turn(0, 1, 0)
        self.assertAlmostEqual(resolve(0, 1, ppo.fight_outcome(1.0, 0.0)), weight)
        self.assertAlmostEqual(rews[0, 0].item(), weight, places=6)
        self.assertEqual(rews[3, 0].item(), 0.0)

        # A tie is zero but still resolves the pending turn.
        end_turn(0, 1, 1)
        self.assertEqual(resolve(0, 1, ppo.fight_outcome(0.0, 0.0)), 0.0)
        self.assertEqual(rews[1, 0].item(), 0.0)
        self.assertEqual(pending[0, 1], -1)

        # A loss.
        end_turn(0, 1, 2)
        self.assertAlmostEqual(resolve(0, 1, ppo.fight_outcome(0.0, 1.0)), -weight)
        self.assertAlmostEqual(rews[2, 0].item(), -weight, places=6)

        # Knocked out in the fight: any place but 1st is a loss.
        end_turn(1, 2, 3)
        self.assertAlmostEqual(
            resolve(1, 2, ppo.place_fight_outcome(5)), -weight)
        self.assertAlmostEqual(rews[3, 1].item(), -weight, places=6)

        # 1st place at the end is a win.
        end_turn(1, 4, 4)
        self.assertAlmostEqual(
            resolve(1, 4, ppo.place_fight_outcome(1)), weight)
        self.assertAlmostEqual(rews[4, 1].item(), weight, places=6)

        # A turn ending on the rollout's last step resolves from the
        # bootstrap observation, still at the turn-end step.
        end_turn(0, 1, 6)
        self.assertAlmostEqual(
            resolve(0, 1, ppo.fight_outcome(1.0, 0.0)), weight)
        self.assertAlmostEqual(rews[6, 0].item(), weight, places=6)

        self.assertEqual(turns, 6)
        self.assertAlmostEqual(rews.sum().item(), weight, places=6)

    def test_no_pending_turn_credits_nothing(self):
        rews = torch.zeros((3, 1))
        pending = np.full((1, 8), -1, np.int64)
        self.assertIsNone(ppo.credit_fight(rews, pending, 0, 0, 1, 0.5))
        self.assertEqual(rews.sum().item(), 0.0)

    def test_outcome_mapping(self):
        self.assertEqual(ppo.fight_outcome(1.0, 0.0), 1)
        self.assertEqual(ppo.fight_outcome(0.0, 1.0), -1)
        self.assertEqual(ppo.fight_outcome(0.0, 0.0), 0)
        self.assertEqual(ppo.place_fight_outcome(1), 1)
        for place in (2, 3, 4, 5, 6, 7, 8):
            self.assertEqual(ppo.place_fight_outcome(place), -1)


if __name__ == "__main__":
    unittest.main()
