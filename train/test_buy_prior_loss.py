"""Tests for the buy-prior loss and agreement (--buy-prior-weight).

    cd train && ../.venv/bin/python -m unittest test_buy_prior_loss
"""

import unittest

import torch

import bgpy
import ppo


def fixture():
    """Four rows: two with a real buy choice, one with a single legal buy and
    one with none. Illegal buy slots carry huge logits, so a missing mask
    would change both answers."""
    slots = ppo.BUY_ACTIONS
    mask = torch.zeros((4, bgpy.N_ACTIONS), dtype=torch.bool)
    for col in (slots[0], slots[2], slots[7]):
        mask[0, col] = True
    for col in (slots[1], slots[3]):
        mask[1, col] = True
    mask[2, slots[5]] = True
    logits = torch.zeros((4, bgpy.N_ACTIONS))
    prior = torch.zeros((4, len(slots)))
    for row in range(4):
        for j, col in enumerate(slots):
            logits[row, col] = (row + 1) * 0.3 + j * 0.7
            prior[row, j] = (4 - row) * 0.2 - j * 0.4
    for row in range(4):
        for j, col in enumerate(slots):
            if not mask[row, col]:
                logits[row, col] = 1e6
                prior[row, j] = -1e6
    return logits, mask, prior


def manual_cross_entropy(logits, mask, prior):
    terms = []
    for row in range(len(mask)):
        legal = [j for j, col in enumerate(ppo.BUY_ACTIONS) if mask[row, col]]
        if len(legal) < 2:
            continue
        target = torch.softmax(prior[row, legal], 0)
        pred = torch.log_softmax(logits[row, [ppo.BUY_ACTIONS[j] for j in legal]], 0)
        terms.append(-(target * pred).sum())
    return torch.stack(terms).mean()


class BuyPriorLossTest(unittest.TestCase):
    def test_zero_weight_is_todays_loss_exactly(self):
        logits, mask, prior = fixture()
        base = torch.tensor(0.25)
        out = ppo.add_buy_prior_loss(base, logits, mask, prior, 0.0)
        self.assertIs(out, base)

    def test_loss_is_the_hand_computed_masked_cross_entropy(self):
        logits, mask, prior = fixture()
        base = torch.tensor(0.25)
        weight = 0.7
        out = ppo.add_buy_prior_loss(base, logits, mask, prior, weight)
        want = base + weight * manual_cross_entropy(logits, mask, prior)
        self.assertTrue(torch.allclose(out, want, atol=1e-6),
                        f"got {out.item()}, want {want.item()}")

    def test_agreement_is_the_direct_masked_argmax_count(self):
        logits, mask, prior = fixture()
        hits, rows = ppo.buy_prior_top1(logits, mask, prior)
        want_hits = want_rows = 0
        for row in range(len(mask)):
            legal = [j for j, col in enumerate(ppo.BUY_ACTIONS) if mask[row, col]]
            if len(legal) < 2:
                continue
            net_best = max(legal, key=lambda j: logits[row, ppo.BUY_ACTIONS[j]].item())
            prior_best = max(legal, key=lambda j: prior[row, j].item())
            want_rows += 1
            want_hits += int(net_best == prior_best)
        self.assertEqual((hits, rows), (want_hits, want_rows))
        self.assertEqual(rows, 2)

    def test_single_or_empty_buy_lists_are_not_decisions(self):
        logits, mask, prior = fixture()
        one = torch.zeros_like(mask)
        one[0, ppo.BUY_ACTIONS[0]] = True
        none = torch.zeros_like(mask)
        for m in (one, none):
            self.assertEqual(ppo.buy_prior_top1(logits, m, prior), (0, 0))
            self.assertEqual(ppo.buy_prior_cross_entropy(logits, m, prior).item(), 0.0)


if __name__ == "__main__":
    unittest.main()
