"""Tests for the placement reward's first and second place bonuses.

    cd train && ../.venv/bin/python -m unittest test_reward
"""

import unittest

import ppo


def old_reward(place):
    return (4.5 - place) / 3.5


class RewardTest(unittest.TestCase):
    def test_zero_bonuses_are_exactly_the_old_reward(self):
        for place in range(1, 9):
            self.assertEqual(ppo.reward(place), old_reward(place))
            self.assertEqual(ppo.reward(place, 0.0, 0.0), old_reward(place))

    def test_first_and_second_bonus_table(self):
        want = [2.00, 1.11, 0.43, 0.14, -0.14, -0.43, -0.71, -1.00]
        for place, value in enumerate(want, 1):
            self.assertAlmostEqual(ppo.reward(place, 1.0, 0.4), value, places=2)

    def test_third_to_eighth_are_unchanged(self):
        for place in range(3, 9):
            self.assertEqual(ppo.reward(place, 1.0, 0.4), old_reward(place))

    def test_bonuses_apply_only_to_their_own_place(self):
        self.assertEqual(ppo.reward(1, 0.0, 0.4), old_reward(1))
        self.assertEqual(ppo.reward(2, 1.0, 0.0), old_reward(2))
        self.assertEqual(ppo.reward(1, 1.0, 0.4), old_reward(1) + 1.0)
        self.assertAlmostEqual(ppo.reward(2, 1.0, 0.4), old_reward(2) + 0.4)


if __name__ == "__main__":
    unittest.main()
