"""A checkpoint's card, hero and trinket rows follow their names when the
tables grow or reorder.

    cd train && ../.venv/bin/python -m unittest test_ckpt_tables
"""

import contextlib
import io
import tempfile
import unittest
from pathlib import Path

import torch
from torch import nn

import model


class Tiny(nn.Module):
    """Only the three embedding tables model.load has to match by name."""

    def __init__(self, n_cards, n_heroes, n_trinkets):
        super().__init__()
        self.card = nn.Embedding(n_cards + 1, 2)
        self.hero = nn.Embedding(n_heroes + 1, 2)
        self.trinket = nn.Embedding(n_trinkets + 1, 2)


class CheckpointTablesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / "tiny.pt"
        self.old = {"cards": ["A", "B", "C"], "heroes": ["h1", "h2", "h3"],
                    "trinkets": ["t1", "t2"]}
        # One card added, two heroes swapped, trinkets unchanged.
        self.new = {"cards": ["A", "B", "C", "D"], "heroes": ["h2", "h1", "h3"],
                    "trinkets": ["t1", "t2"]}
        src = Tiny(len(self.old["cards"]), len(self.old["heroes"]), len(self.old["trinkets"]))
        with torch.no_grad():
            for table in (src.card, src.hero, src.trinket):
                for row in range(table.num_embeddings):
                    table.weight[row] = torch.tensor([float(row), 100.0 + row])
        torch.save({"net": src.state_dict(), "model": "tiny", "model_opts": {},
                    "card_names": self.old["cards"], "hero_names": self.old["heroes"],
                    "trinket_names": self.old["trinkets"]}, self.path)
        self.want = {k: v.clone() for k, v in src.state_dict().items()}

        saved_build, saved_names = model.build, model.current_names
        model.build = lambda kind, compile=False, **opts: Tiny(
            len(self.new["cards"]), len(self.new["heroes"]), len(self.new["trinkets"]))
        model.current_names = lambda: self.new
        self.addCleanup(setattr, model, "build", saved_build)
        self.addCleanup(setattr, model, "current_names", saved_names)

    def test_known_rows_follow_names_and_new_names_take_the_empty_row(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            net = model.load(str(self.path))
        sd = net.state_dict()

        # The card added at the end gets the old table's empty row (0).
        for i, name in enumerate(self.new["cards"]):
            row = self.old["cards"].index(name) + 1 if name in self.old["cards"] else 0
            self.assertTrue(torch.equal(sd["card.weight"][i + 1], self.want["card.weight"][row]),
                            f"card {name}")
        # Swapping the two hero names swaps their rows.
        for i, name in enumerate(self.new["heroes"]):
            row = self.old["heroes"].index(name) + 1
            self.assertTrue(torch.equal(sd["hero.weight"][i + 1], self.want["hero.weight"][row]),
                            f"hero {name}")
        # Unchanged trinkets are left exactly as they were.
        self.assertTrue(torch.equal(sd["trinket.weight"], self.want["trinket.weight"]))
        self.assertTrue(torch.equal(sd["card.weight"][0], self.want["card.weight"][0]))

        printed = out.getvalue()
        self.assertIn("load", printed)
        self.assertIn("cards: 3 kept, 1 new, 0 dropped", printed)
        self.assertIn("heroes: 3 kept, 0 new, 0 dropped", printed)
        self.assertNotIn("trinkets:", printed)

    def test_repeated_names_match_by_occurrence(self):
        # Lesser and greater trinkets can share a name; each row follows its
        # occurrence, not the name's last position.
        src = torch.arange(8, dtype=torch.float32).reshape(4, 2)
        own = torch.zeros(5, 2)
        w, kept, fresh, dropped = model._remap(
            src, own, ["t1", "t2", "t1"], ["t1", "t1", "t2", "t3"], base=0)
        self.assertTrue(torch.equal(w[1], src[1]))
        self.assertTrue(torch.equal(w[2], src[3]))
        self.assertTrue(torch.equal(w[3], src[2]))
        self.assertTrue(torch.equal(w[4], src[0]))
        self.assertEqual((kept, fresh, dropped), (3, 1, 0))


if __name__ == "__main__":
    unittest.main()
