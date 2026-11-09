"""Tests for card text tokens and the network's text input.

    cd train && ../.venv/bin/python -m unittest test_card_text
"""

import tempfile
import unittest
from pathlib import Path

import torch

import bgpy
import card_text
import model
from model import build, ck_model, load


class TokenizeTest(unittest.TestCase):
    def test_markup_and_keywords(self):
        text = "<b>Taunt</b>. <b>Deathrattle:</b> Summon a 2/3 Beast."
        self.assertEqual(card_text.tokenize(text),
                         ["taunt", "deathrattle", "summon", "a", "<num>", "<num>", "beast"])

    def test_buff_is_one_token(self):
        self.assertEqual(card_text.tokenize("Give a minion +7/+14 until next turn."),
                         ["give", "a", "minion", "<buff>", "until", "next", "turn"])
        self.assertEqual(card_text.tokenize("+2/ +3"), ["<buff>"])

    def test_counters_are_dropped(self):
        text = "After you buy 3 minions, get a Blood Gem. <i>(3 left!)</i>"
        self.assertEqual(card_text.tokenize(text),
                         ["after", "you", "buy", "<num>", "minions", "get", "a",
                          "blood", "gem"])
        gold = "[x]Whenever you spend 3 Gold, get a Blood Gem. <i>(3 Gold left!)</i>"
        self.assertNotIn("left", card_text.tokenize(gold))

    def test_bracket_tags_and_empty(self):
        self.assertEqual(card_text.tokenize("[x]<b>Divine Shield</b>\n<b>Windfury</b>"),
                         ["divine", "shield", "windfury"])
        self.assertEqual(card_text.tokenize(""), [])
        self.assertEqual(card_text.tokenize(None), [])

    def test_number_and_apostrophe(self):
        self.assertEqual(card_text.tokenize("It doesn\u2019t give a Triple Reward."),
                         ["it", "doesn't", "give", "a", "triple", "reward"])


class TableTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        env = bgpy.VecEnv(1, 0, [8, 0, 0, 0, 0, 0])
        cls.names = env.card_names()
        cls.table = card_text.build_table(cls.names)

    def test_shape_and_padding(self):
        rows = self.table["tokens"]
        self.assertEqual(len(rows), len(self.names) + 1)
        self.assertTrue(all(len(row) == card_text.MAX_TOKENS for row in rows))
        self.assertEqual(rows[0], [0] * card_text.MAX_TOKENS)
        for row in rows[1:]:
            last = max((i for i, token in enumerate(row) if token), default=-1)
            self.assertTrue(all(token == 0 for token in row[last + 1:]))

    def test_identical_text_identical_rows(self):
        i = self.names.index("Deadly Spore") + 1
        j = self.names.index("Amalgam") + 1
        self.assertEqual(self.table["tokens"][i], self.table["tokens"][j])
        self.assertTrue(any(self.table["tokens"][i]))

    def test_textless_card_is_empty(self):
        i = self.names.index("Stand-in") + 1
        self.assertEqual(self.table["tokens"][i], [0] * card_text.MAX_TOKENS)
        self.assertIn("Stand-in", self.table["missing"])


class ModelTextTest(unittest.TestCase):
    opts = dict(opp_boards="next", d=16, layers=1, heads=2, ff=32, emb=8, n_comps=0)

    @classmethod
    def setUpClass(cls):
        env = bgpy.VecEnv(1, 0, [8, 0, 0, 0, 0, 0])
        cls.names = env.card_names()
        cls.table = card_text.build_table(cls.names)
        cls.card_text = cls.table

    def build(self, card_text=None):
        return build("attn", n_cards=bgpy.N_CARDS, card_text=card_text, **self.opts)

    def test_text_off_is_unchanged(self):
        torch.manual_seed(0)
        a = self.build()
        torch.manual_seed(0)
        b = self.build(card_text=None)
        self.assertEqual(set(a.state_dict()), set(b.state_dict()))
        for name, value in a.state_dict().items():
            self.assertTrue(torch.equal(value, b.state_dict()[name]), name)
        self.assertFalse(any(name.startswith("inp.text.") for name in a.state_dict()))

    def test_text_on_adds_the_table_and_shares_text_vectors(self):
        net = self.build(card_text=self.table)
        self.assertIsNotNone(net.inp.text)
        vectors = net.inp.text.table()
        i = self.names.index("Deadly Spore") + 1
        j = self.names.index("Amalgam") + 1
        self.assertTrue(torch.equal(vectors[i], vectors[j]))
        self.assertTrue(vectors[i].abs().sum() > 0)
        self.assertEqual(vectors[self.names.index("Stand-in") + 1].abs().sum().item(), 0.0)

    def test_checkpoint_round_trip_keeps_output(self):
        net = self.build(card_text=self.table).eval()
        obs = torch.zeros(2, bgpy.OBS_SIZE)
        with torch.no_grad():
            want = net(obs)
        ck = {"net": net.state_dict(), "model": "attn",
              "model_opts": {"opp_boards": "next", "d": 16, "layers": 1,
                             "heads": 2, "ff": 32, "emb": 8, "n_comps": 0,
                             "card_text": self.table}}
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "ckpt.pt"
            torch.save(ck, path)
            kind, opts = ck_model(torch.load(path, map_location="cpu"))
            self.assertEqual(kind, "attn")
            self.assertEqual(opts["card_text"]["vocab"], self.table["vocab"])
            got = load(str(path)).eval()
            with torch.no_grad():
                out = got(obs)
        for a, b in zip(want, out):
            self.assertTrue(torch.allclose(a, b))

    def test_old_checkpoint_without_text_still_loads(self):
        torch.manual_seed(0)
        net = self.build()
        obs = torch.zeros(1, bgpy.OBS_SIZE)
        with torch.no_grad():
            want = net(obs)
        ck = {"net": net.state_dict(), "model": "attn",
              "model_opts": {"opp_boards": "next", "d": 16, "layers": 1,
                             "heads": 2, "ff": 32, "emb": 8, "n_comps": 0}}
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "ckpt.pt"
            torch.save(ck, path)
            _, opts = ck_model(torch.load(path, map_location="cpu"))
            self.assertIsNone(opts["card_text"])
            got = load(str(path)).eval()
            with torch.no_grad():
                out = got(obs)
        for a, b in zip(want, out):
            self.assertTrue(torch.allclose(a, b))

    def test_mlp_and_pool_also_read_text(self):
        for kind in ("mlp", "pool"):
            net = build(kind, n_cards=bgpy.N_CARDS, card_text=self.table)
            with torch.no_grad():
                net(torch.zeros(2, bgpy.OBS_SIZE))
            if kind == "mlp":
                self.assertIsNotNone(net.text)
            else:
                self.assertIsNotNone(net.inp.text)


if __name__ == "__main__":
    unittest.main()
