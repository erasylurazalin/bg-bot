"""Tests for the kind-level demonstration loss (--demo-kind-weight).

    cd train && ../.venv/bin/python -m unittest test_demo_kind
"""

import unittest

import torch

import bgpy
import ppo
from test_demo_loss import TinyPolicy, fixture


class DemoKindTest(unittest.TestCase):
    def test_grouped_probabilities_sum_to_one_over_legal_kinds(self):
        obs, mask, _, _ = fixture()
        policy = TinyPolicy(obs.shape[1])
        logits, _ = policy(obs)
        probs = ppo.demo_kind_probs(logits, mask)
        self.assertEqual(probs.shape,
                         (len(obs), len(ppo.DEMO_KIND_GROUPS)))
        self.assertTrue(torch.allclose(probs.sum(1), torch.ones(len(obs)),
                                       atol=1e-6))

    def test_grouped_probabilities_are_the_direct_sums(self):
        obs, mask, _, _ = fixture()
        policy = TinyPolicy(obs.shape[1])
        logits, _ = policy(obs)
        probs = torch.softmax(ppo.masked(logits, mask), -1)
        got = ppo.demo_kind_probs(logits, mask)
        for group in range(len(ppo.DEMO_KIND_GROUPS)):
            legal = mask & (ppo.DEMO_KIND_OF == group).unsqueeze(0)
            want = probs.masked_fill(~legal, 0.0).sum(1)
            self.assertTrue(torch.allclose(got[:, group], want, atol=1e-6),
                            f"group {ppo.DEMO_KIND_GROUPS[group]}")

    def test_every_action_kind_is_grouped(self):
        self.assertEqual(len(ppo.DEMO_KIND_OF), bgpy.N_ACTIONS)
        self.assertEqual(len(ppo.DEMO_KIND_OF_KIND), len(ppo.KINDS))
        self.assertEqual(set(ppo.DEMO_KIND_OF.tolist()),
                         set(range(len(ppo.DEMO_KIND_GROUPS))))

    def test_zero_weight_is_todays_loss_exactly(self):
        obs, mask, action, _ = fixture()
        policy = TinyPolicy(obs.shape[1])
        base = torch.tensor(0.25)
        out = ppo.add_demo_kind_loss(base, policy, obs, mask, action, 0.0)
        self.assertIs(out, base)

    def test_added_term_is_the_hand_computed_kind_cross_entropy(self):
        obs, mask, action, _ = fixture()
        policy = TinyPolicy(obs.shape[1])
        base = torch.tensor(0.25)
        weight = 0.7
        out = ppo.add_demo_kind_loss(base, policy, obs, mask, action, weight)
        logits, _ = policy(obs)
        logp = torch.log_softmax(ppo.masked(logits, mask), -1)
        target = ppo.DEMO_KIND_OF[action]
        terms = []
        for row, kind in enumerate(target):
            legal = mask[row] & (ppo.DEMO_KIND_OF == kind)
            terms.append(-logp[row][legal].exp().sum().log())
        want = base + weight * torch.stack(terms).mean()
        self.assertTrue(torch.allclose(out, want, atol=1e-6),
                        f"got {out.item()}, want {want.item()}")

    def test_kind_agreement_is_the_direct_argmax_count(self):
        obs, mask, action, split = fixture()
        policy = TinyPolicy(obs.shape[1])
        demo = ppo.DemoData(obs, mask, action, split)

        got = ppo.demo_agreement(policy, demo, "cpu", batch=2)
        logits, _ = policy(demo.obs)
        pred = ppo.demo_kind_probs(logits, demo.mask).argmax(1)
        hit = pred == demo.kind
        held = demo.heldout
        self.assertAlmostEqual(got["demo_kind_top1"],
                               float(hit[held].float().mean()))
        self.assertAlmostEqual(got["demo_kind_top1_train"],
                               float(hit[demo.train_fixed].float().mean()))


if __name__ == "__main__":
    unittest.main()
