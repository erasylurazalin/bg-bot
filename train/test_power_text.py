"""Tests for hero power, trinket and Dark Gift text.

    cd train && ../.venv/bin/python -m unittest test_power_text
"""

import tempfile
import unittest
from pathlib import Path

import torch

import bgpy
import card_text
import model
from model import build, load, text_gates, warm_start


class TextTableTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.env = bgpy.VecEnv(1, 0, [8, 0, 0, 0, 0, 0])
        cls.names = cls.env.card_names()
        cls.heroes = cls.env.hero_names()
        cls.trinkets = cls.env.trinket_names()
        cls.cards, cls.powers = card_text.build_text(cls.names, cls.heroes, cls.trinkets)

    def test_one_shared_vocabulary(self):
        self.assertEqual(self.cards["vocab"], self.powers["vocab"])
        self.assertEqual(len(self.cards["tokens"]), len(self.names) + 1)
        self.assertEqual(len(self.powers["powers"]), len(self.heroes) + 1)
        self.assertEqual(len(self.powers["trinkets"]), len(self.trinkets) + 1)

    def test_known_power_row(self):
        # Malygos's Arcane Alteration, by the TOML's engine order.
        i = card_text.power_names().index("Arcane Alteration")
        self.assertEqual(self.heroes[i], "Malygos")
        row = self.powers["powers"][i + 1]
        self.assertTrue(any(row))
        self.assertEqual(row[0], self.powers["vocab"].index("replace") + 1)
        self.assertIn(0, row)        # padded
        self.assertEqual(self.powers["powers"][0], [0] * card_text.MAX_TOKENS)

    def test_known_trinket_row(self):
        i = self.trinkets.index("Chromatic Tear")
        row = self.powers["trinkets"][i + 1]
        self.assertTrue(any(row))
        self.assertIn("chromadrake", card_text.tokenize(
            card_text.read_id_texts([card_text.trinket_ids()[i]])[0][
                card_text.trinket_ids()[i]]))

    def test_dark_gift_is_a_card_row(self):
        # A gift reaches the network as its effect card's id in the main card
        # table (the gift slots on minions, and a Discover slot's gift field
        # during an offer), never as an id of its own.
        card_id, token = card_text.gift_entries()[0]
        self.assertTrue((card_text.read_id_texts([card_id])[0][card_id]))
        i = self.names.index(token)
        self.assertTrue(any(self.cards["tokens"][i + 1]))
        gifts, missing = card_text.build_gift_report()
        self.assertEqual(len(missing), 0)
        self.assertEqual(gifts, 47)


class GateZeroTest(unittest.TestCase):
    opts = dict(opp_boards="next", d=16, layers=1, heads=2, ff=32, emb=8, n_comps=0)

    @classmethod
    def setUpClass(cls):
        cls.env = bgpy.VecEnv(1, 0, [8, 0, 0, 0, 0, 0])
        cls.names = cls.env.card_names()
        cls.heroes = cls.env.hero_names()
        cls.trinkets = cls.env.trinket_names()
        cls.cards = card_text.build_table(cls.names)
        cls.cards.update(card_text.build_forms(cls.names))
        torch.manual_seed(0)
        cls.net = build("attn", len(cls.names), card_text=cls.cards, **cls.opts).eval()
        cls.obs = cls.batch()
        with torch.no_grad():
            cls.want = cls.net(cls.obs)
        cls.tmp = tempfile.TemporaryDirectory()
        cls.path = Path(cls.tmp.name) / "ckpt.pt"
        torch.save({"net": cls.net.state_dict(), "model": "attn",
                    "model_opts": dict(cls.opts, card_text=cls.cards),
                    "card_names": cls.names, "hero_names": cls.heroes,
                    "trinket_names": cls.trinkets}, cls.path)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    @classmethod
    def batch(cls):
        """Observations with a card slot, a scalar card, heroes and trinkets
        all non-zero, so every text path is actually read."""
        obs = torch.zeros(3, bgpy.OBS_SIZE)
        card = cls.names.index("Deadly Spore") + 1
        obs[:, model.SCALAR_IDS[0]] = card
        slot = bgpy.SCALARS
        obs[:, slot + bgpy.SLOT_CARD_ID] = card
        obs[:, slot + bgpy.SLOT_PRESENT] = 1
        obs[:, slot + bgpy.SLOT_TAUGHT_SPELL] = cls.names.index("Blood Gem") + 1
        opp = bgpy.OWN_END + bgpy.OPP_SCALARS
        obs[:, opp + bgpy.SLOT_CARD_ID] = card
        obs[:, opp + bgpy.SLOT_PRESENT] = 1
        power = card_text.power_names().index("Arcane Alteration") + 1
        trinket = cls.trinkets.index("Chromatic Tear") + 1
        obs[:, model.HERO_IDS[0]] = power
        obs[:, bgpy.OWN_END + model.OPP_IDS[0]] = power
        obs[:, model.TRINKET_IDS[0]] = trinket
        obs[:, bgpy.OWN_END + model.OPP_TRINKET_IDS[0]] = trinket
        return obs

    def assert_same(self, net):
        with torch.no_grad():
            got = net(self.obs)
        for want, out in zip(self.want, got):
            self.assertTrue(torch.equal(want, out))

    def test_load_power_text_changes_nothing_at_gate_zero(self):
        plain = load(str(self.path)).eval()
        powered = load(str(self.path), power_text=True).eval()
        self.assertEqual(text_gates(plain), [])
        self.assertEqual(text_gates(powered), [("hero power", 0.0), ("trinket", 0.0)])
        self.assertEqual(len(plain.inp.text.vocab), len(self.cards["vocab"]))
        self.assertGreater(len(powered.inp.text.vocab), len(self.cards["vocab"]))
        self.assert_same(plain)
        self.assert_same(powered)
        self.assertTrue(any(powered.inp.text.power_tokens[0] == 0))
        self.assertTrue(powered.inp.text.power_table().abs().sum() > 0)

    def test_warm_start_power_text_changes_nothing_at_gate_zero(self):
        cards, powers = card_text.build_text(self.names, self.heroes, self.trinkets)
        cards.update(card_text.build_forms(self.names))
        net = build("attn", len(self.names), card_text=cards, power_text=powers,
                    **self.opts).eval()
        loaded, fresh, _, _, _ = warm_start(
            net, "attn", str(self.path), cards=self.names, heroes=self.heroes,
            trinkets=self.trinkets)
        self.assertIn("inp.text.power_gate", fresh)
        self.assertIn("inp.text.trinket_gate", fresh)
        self.assertEqual(text_gates(net), [("hero power", 0.0), ("trinket", 0.0)])
        self.assertTrue(loaded)
        self.assert_same(net)

    def test_power_text_without_card_text(self):
        _, powers = card_text.build_text(self.names, self.heroes, self.trinkets)
        net = build("mlp", len(self.names), power_text=powers).eval()
        with torch.no_grad():
            net(self.obs)
        self.assertIsNotNone(net.text)
        self.assertFalse(net.text.has_cards)
        self.assertEqual(net.text.power_table().shape[0], len(self.heroes) + 1)


if __name__ == "__main__":
    unittest.main()
