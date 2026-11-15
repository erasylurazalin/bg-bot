"""Tests for the guarded cycle count in train/evaltools/turn_motion.py.

    cd train && ../.venv/bin/python -m unittest test_turn_motion
"""

import collections
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "evaltools"))

import turn_motion as tm

FIXTURE = """
[[minion]]
name = "Battlecry Cycled"
tier = 1
tribe = "neutral"
stats = [1, 1]
battlecry = { get_gems = 1 }

[[minion]]
name = "Seller Cycled"
tier = 2
tribe = "neutral"
stats = [2, 2]
on_sell = { effect = { spell_bonus = [1, 1] } }

[[minion]]
name = "Plain Cycled"
tier = 1
tribe = "neutral"
stats = [1, 2]

[[token]]
name = "Plain Token"
tribe = "neutral"
stats = [1, 1]
"""


class CycleFxTest(unittest.TestCase):
    def test_battlecry_and_sell_effects_count_but_a_plain_cycle_does_not(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "cards.toml"
            path.write_text(FIXTURE)
            effects = tm.effect_cards(path)
        self.assertIn("Battlecry Cycled", effects)
        self.assertIn("Seller Cycled", effects)
        self.assertNotIn("Plain Cycled", effects)
        self.assertNotIn("Plain Token", effects)

        tot = collections.Counter()
        by_card = collections.Counter()
        self.assertTrue(tm.note_cycle(
            tot, by_card, "Battlecry Cycled", 10, effects))
        self.assertTrue(tm.note_cycle(
            tot, by_card, "Seller Cycled", 11, effects))
        self.assertFalse(tm.note_cycle(
            tot, by_card, "Plain Cycled", 10, effects))
        self.assertEqual(tot[("cycles", 10)], 2)
        self.assertEqual(tot[("cycles", 11)], 1)
        self.assertEqual(tot[("cycles_fx", 10)], 1)
        self.assertEqual(tot[("cycles_fx", 11)], 1)
        self.assertEqual(by_card["Plain Cycled"], 1)


if __name__ == "__main__":
    unittest.main()
