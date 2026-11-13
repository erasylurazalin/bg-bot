"""Tests for the demonstration loss (--demo/--demo-weight/--demo-fade).

    cd train && ../.venv/bin/python -m unittest test_demo_loss
"""

import tempfile
import unittest
from pathlib import Path

import numpy as np
import torch

import bgpy
import ppo


class TinyPolicy(torch.nn.Module):
    def __init__(self, width):
        super().__init__()
        self.fc = torch.nn.Linear(width, bgpy.N_ACTIONS)

    def forward(self, obs, comp=None):
        return self.fc(obs), obs[:, :1].squeeze(1)


def fixture():
    """Six rows: three train, three held out, one held-out buy and one
    held-out non-buy. Width 4 keeps the test's network tiny."""
    torch.manual_seed(7)
    buy = ppo.KINDS.index("buy")
    lo = sum(count for _, count in bgpy.ACTION_KINDS[:buy])
    obs = torch.arange(24, dtype=torch.float32).reshape(6, 4) / 10
    mask = torch.ones((6, bgpy.N_ACTIONS), dtype=torch.bool)
    mask[:, 0] = False
    action = torch.tensor([lo, 5, 40, lo + 1, 60, 100])
    split = np.array(["train", "train", "train",
                      "heldout", "heldout", "heldout"])
    return obs, mask, action, split


class DemoLossTest(unittest.TestCase):
    def test_zero_weight_is_todays_loss_exactly(self):
        obs, mask, action, _ = fixture()
        policy = TinyPolicy(obs.shape[1])
        base = torch.tensor(0.25)
        out = ppo.add_demo_loss(base, policy, obs, mask, action, 0.0)
        self.assertIs(out, base)

    def test_added_term_is_the_hand_computed_cross_entropy(self):
        obs, mask, action, _ = fixture()
        policy = TinyPolicy(obs.shape[1])
        base = torch.tensor(0.25)
        weight = 0.7
        out = ppo.add_demo_loss(base, policy, obs, mask, action, weight)
        logits, _ = policy(obs)
        logp = torch.log_softmax(logits.masked_fill(~mask, -1e9), dim=1)
        want = base + weight * (-logp.gather(1, action[:, None]).mean())
        self.assertTrue(torch.allclose(out, want, atol=1e-6),
                        f"got {out.item()}, want {want.item()}")

    def test_fade_falls_to_zero_at_the_planned_end(self):
        f = ppo.demo_fade_scale
        self.assertEqual(f(10, 0, 100, 0.0), 1.0)
        self.assertEqual(f(74, 0, 100, 0.25), 1.0)
        self.assertAlmostEqual(f(87.5, 0, 100, 0.25), 0.5)
        self.assertEqual(f(100, 0, 100, 0.25), 0.0)
        self.assertEqual(f(120, 0, 100, 0.25), 0.0)
        self.assertAlmostEqual(f(80, 50, 110, 0.5), 1.0)
        self.assertAlmostEqual(f(95, 50, 110, 0.5), 0.5)
        self.assertEqual(f(110, 50, 110, 0.5), 0.0)

    def test_agreement_is_the_direct_masked_argmax_count(self):
        obs, mask, action, split = fixture()
        policy = TinyPolicy(obs.shape[1])
        demo = ppo.DemoData(obs, mask, action, split)
        self.assertEqual(len(demo.train_fixed), len(demo.heldout))

        got = ppo.demo_agreement(policy, demo, "cpu", batch=2)
        pred = ppo.masked(policy(demo.obs)[0], demo.mask).argmax(1)
        hit = pred == demo.action
        held = demo.heldout
        held_buy = held[demo.is_buy[held]]
        self.assertGreater(len(held_buy), 0, "the fixture has a held-out buy")
        self.assertAlmostEqual(got["demo_top1"], float(hit[held].float().mean()))
        self.assertAlmostEqual(got["demo_top1_buy"],
                               float(hit[held_buy].float().mean()))
        self.assertAlmostEqual(got["demo_top1_train"],
                               float(hit[demo.train_fixed].float().mean()))

    def test_loader_refuses_a_different_observation_width(self):
        obs, mask, action, split = fixture()
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "demo.npz"
            np.savez(path, obs=obs.numpy(), mask=mask.numpy(),
                     action=action.numpy(), split=split)
            with self.assertRaises(SystemExit) as cm:
                ppo.load_demo(path, obs.shape[1] + 1)
            self.assertIn("wide", str(cm.exception))


if __name__ == "__main__":
    unittest.main()
