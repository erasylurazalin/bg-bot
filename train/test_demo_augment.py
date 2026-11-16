"""Tests for the `--demo-augment` shuffles and opponent-board swap.

    cd train && ../.venv/bin/python -m unittest test_demo_augment

CPU only. The rows are hand-built observations: known card ids in every shop,
hand and board slot, and known alternative opponent patches.
"""

import tempfile
import unittest
from pathlib import Path
import sys

import numpy as np
import torch

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))

import bgpy
import ppo


SF = bgpy.SLOT_FEATURES
PATCH = ppo.DEMO_ALT_OPP


def blank_obs(rows=1):
    """Shop, hand and board slots with a unique card id each; opponent 2 is
    the next opponent with its own board. Everything else is zero."""
    obs = torch.zeros((rows, bgpy.OBS_SIZE), dtype=torch.float32)
    for row in range(rows):
        for i in range(bgpy.SHOP):
            at = bgpy.SHOP_START + i * SF
            obs[row, at + bgpy.SLOT_PRESENT] = 1.0
            obs[row, at + bgpy.SLOT_CARD_ID] = 100.0 + i
        for i in range(bgpy.HAND):
            at = bgpy.HAND_START + i * SF
            obs[row, at + bgpy.SLOT_PRESENT] = 1.0
            obs[row, at + bgpy.SLOT_CARD_ID] = 200.0 + i
        for i in range(bgpy.BOARD):
            at = bgpy.BOARD_START + i * SF
            obs[row, at + bgpy.SLOT_PRESENT] = 1.0
            obs[row, at + bgpy.SLOT_CARD_ID] = 300.0 + i
        next_at = bgpy.OWN_END + 2 * bgpy.OPP_BLOCK
        obs[row, next_at + bgpy.OPP_NEXT] = 1.0
        for i in range(bgpy.BOARD):
            at = next_at + bgpy.OPP_SCALARS + i * SF
            obs[row, at + bgpy.SLOT_PRESENT] = 1.0
            obs[row, at + bgpy.SLOT_CARD_ID] = 400.0 + i
        for i in range(bgpy.LOBBY_TRIBES):
            obs[row, next_at + bgpy.OPP_TRIBES + i] = 10.0 + i
    return obs


def alt_patches(rows=1, k=3):
    """Distinct patches with recognisable board card ids and tribe counts."""
    alt = torch.zeros((rows, k, PATCH), dtype=torch.float32)
    for row in range(rows):
        for j in range(k):
            base = 1000.0 * (row + 1) + 100.0 * j
            for slot in range(bgpy.BOARD):
                at = slot * bgpy.MINION_FEATURES
                alt[row, j, at + bgpy.SLOT_PRESENT] = 1.0
                alt[row, j, at + bgpy.SLOT_CARD_ID] = base + slot
            alt[row, j, bgpy.BOARD * bgpy.MINION_FEATURES:] = (
                20.0 + 10.0 * j + torch.arange(bgpy.LOBBY_TRIBES)
            )
    return alt


def shop_action(slot):
    return ppo.BUY_BASE + slot


def play_action(hand, pos):
    return ppo.PLAY_BASE + hand * (bgpy.BOARD + 1) + pos


def discard_action(hand):
    return ppo.DISCARD_BASE + hand


class DemoAugmentTest(unittest.TestCase):
    def test_shop_shuffle_keeps_the_same_card_and_permutes_the_mask(self):
        obs = blank_obs()
        mask = torch.zeros((1, bgpy.N_ACTIONS), dtype=torch.bool)
        old_slot = 4
        mask[0, shop_action(old_slot)] = True
        mask[0, ppo.TARGET_SHOP_BASE + 5] = True
        action = torch.tensor([shop_action(old_slot)])
        src = torch.tensor([[1, 2, 3, 4, 5, 6, 0]])
        old_mask = mask.clone()
        old_obs = obs.clone()

        ppo.apply_shop_shuffle(obs, mask, action, src)

        new_slot = int(torch.argsort(src, dim=1)[0, old_slot])
        self.assertEqual(int(action[0]), shop_action(new_slot))
        card = obs[0, bgpy.SHOP_START + new_slot * SF + bgpy.SLOT_CARD_ID]
        self.assertEqual(float(card), 100.0 + old_slot)
        # Every buy and target-shop bit moved with its slot.
        dst = torch.argsort(src, dim=1)
        for old in range(bgpy.SHOP):
            new = int(dst[0, old])
            self.assertEqual(
                bool(mask[0, shop_action(new)]),
                bool(old_mask[0, shop_action(old)]))
            self.assertEqual(
                bool(mask[0, ppo.TARGET_SHOP_BASE + new]),
                bool(old_mask[0, ppo.TARGET_SHOP_BASE + old]))
        # Nothing outside the shop slots moved.
        changed = torch.zeros(bgpy.OBS_SIZE, dtype=torch.bool)
        changed[bgpy.SHOP_START:bgpy.SHOP_START + bgpy.SHOP * SF] = True
        self.assertTrue(torch.equal(obs[0, ~changed], old_obs[0, ~changed]))

    def test_hand_shuffle_keeps_the_same_card_and_permutes_the_mask(self):
        obs = blank_obs()
        mask = torch.zeros((1, bgpy.N_ACTIONS), dtype=torch.bool)
        old_hand = 4
        pos = bgpy.BOARD  # play without a board position
        mask[0, play_action(old_hand, pos)] = True
        mask[0, discard_action(6)] = True
        action = torch.tensor([play_action(old_hand, pos)])
        src = torch.tensor([[1, 2, 3, 4, 5, 6, 7, 8, 9, 0]])
        old_mask = mask.clone()
        old_obs = obs.clone()

        ppo.apply_hand_shuffle(obs, mask, action, src)

        new_hand = int(torch.argsort(src, dim=1)[0, old_hand])
        self.assertEqual(int(action[0]), play_action(new_hand, pos))
        card = obs[0, bgpy.HAND_START + new_hand * SF + bgpy.SLOT_CARD_ID]
        self.assertEqual(float(card), 200.0 + old_hand)
        dst = torch.argsort(src, dim=1)
        for old in range(bgpy.HAND):
            new = int(dst[0, old])
            for p in range(bgpy.BOARD + 1):
                self.assertEqual(
                    bool(mask[0, play_action(new, p)]),
                    bool(old_mask[0, play_action(old, p)]))
            self.assertEqual(
                bool(mask[0, discard_action(new)]),
                bool(old_mask[0, discard_action(old)]))
        changed = torch.zeros(bgpy.OBS_SIZE, dtype=torch.bool)
        changed[bgpy.HAND_START:bgpy.HAND_START + bgpy.HAND * SF] = True
        self.assertTrue(torch.equal(obs[0, ~changed], old_obs[0, ~changed]))

    def test_opponent_swap_is_one_stored_alternative_and_nothing_else(self):
        obs = blank_obs(rows=2)
        alt = alt_patches(rows=2, k=3)
        picks = torch.tensor([0, 2])
        old = obs.clone()
        ppo.apply_opponent_swap(obs, alt, picks)

        changed = torch.zeros((2, bgpy.OBS_SIZE), dtype=torch.bool)
        for row, j in enumerate(picks.tolist()):
            at = bgpy.OWN_END + 2 * bgpy.OPP_BLOCK
            changed[row, at + bgpy.OPP_SCALARS:
                    at + bgpy.OPP_SCALARS + bgpy.BOARD * SF] = True
            changed[row, at + bgpy.OPP_TRIBES:
                    at + bgpy.OPP_TRIBES + bgpy.LOBBY_TRIBES] = True
            got = torch.cat([
                obs[row, at + bgpy.OPP_SCALARS:
                    at + bgpy.OPP_SCALARS + bgpy.BOARD * SF],
                obs[row, at + bgpy.OPP_TRIBES:
                    at + bgpy.OPP_TRIBES + bgpy.LOBBY_TRIBES],
            ])
            self.assertTrue(torch.equal(got, alt[row, j]))
        self.assertTrue(torch.equal(obs[~changed], old[~changed]))
        # The learner's own board is not part of any augmentation.
        self.assertTrue(torch.equal(
            obs[:, bgpy.BOARD_START:bgpy.BOARD_START + bgpy.BOARD * SF],
            old[:, bgpy.BOARD_START:bgpy.BOARD_START + bgpy.BOARD * SF]))

    def test_no_next_opponent_means_no_swap(self):
        obs = blank_obs(rows=1)
        at = bgpy.OWN_END + 2 * bgpy.OPP_BLOCK
        obs[0, at + bgpy.OPP_NEXT] = 0.0
        old = obs.clone()
        ppo.apply_opponent_swap(obs, alt_patches(k=3), torch.tensor([1]))
        self.assertTrue(torch.equal(obs, old))

    def test_an_empty_logged_board_is_not_invented(self):
        obs = blank_obs(rows=1)
        at = bgpy.OWN_END + 2 * bgpy.OPP_BLOCK
        for slot in range(bgpy.BOARD):
            start = at + bgpy.OPP_SCALARS + slot * SF
            obs[0, start:start + SF] = 0.0
        old = obs.clone()
        ppo.apply_opponent_swap(obs, alt_patches(k=3), torch.tensor([1]))
        self.assertTrue(torch.equal(obs, old))

    def test_full_augmentation_leaves_the_learner_board_alone(self):
        demo = ppo.DemoData(
            blank_obs(rows=4),
            torch.zeros((4, bgpy.N_ACTIONS), dtype=bool),
            torch.zeros(4, dtype=torch.long),
            np.asarray(["train", "train", "heldout", "heldout"]),
            alt_opp=alt_patches(rows=4, k=3),
        )
        before = demo.obs.clone()
        rows = torch.arange(4)
        generator = torch.Generator().manual_seed(7)
        obs, _mask, _action = ppo.augment_demo_batch(demo, rows, generator)
        board = slice(bgpy.BOARD_START, bgpy.BOARD_START + bgpy.BOARD * SF)
        self.assertTrue(torch.equal(obs[:, board], before[:, board]))
        # The source dataset is never changed in place.
        self.assertTrue(torch.equal(demo.obs, before))

    def test_off_draw_is_the_same_for_the_same_seed(self):
        demo = ppo.DemoData(
            blank_obs(rows=4),
            torch.zeros((4, bgpy.N_ACTIONS), dtype=bool),
            torch.zeros(4, dtype=torch.long),
            np.asarray(["train", "train", "heldout", "heldout"]),
        )
        first = ppo.demo_rows_for_minibatch(
            demo, 3, torch.Generator().manual_seed(99))
        second = ppo.demo_rows_for_minibatch(
            demo, 3, torch.Generator().manual_seed(99))
        self.assertTrue(torch.equal(first, second))
        manual = demo.train[torch.randint(
            len(demo.train), (3,), generator=torch.Generator().manual_seed(99))]
        self.assertTrue(torch.equal(first, manual))
        # The old draw is exactly these rows and fields, not a copy.
        self.assertTrue(torch.equal(demo.obs[first], demo.obs[first].clone()))
        self.assertTrue(torch.equal(demo.mask[first], demo.mask[first].clone()))
        self.assertTrue(torch.equal(demo.action[first], demo.action[first].clone()))

    def test_augment_needs_the_alternative_array(self):
        obs = blank_obs(rows=2).numpy()
        mask = np.zeros((2, bgpy.N_ACTIONS), dtype=bool)
        action = np.zeros(2, dtype=np.int64)
        split = np.asarray(["train", "heldout"])
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "demo.npz"
            np.savez(path, obs=obs, mask=mask, action=action, split=split)
            with self.assertRaises(SystemExit) as caught:
                ppo.load_demo(path, bgpy.OBS_SIZE, augment=True)
            message = str(caught.exception)
            self.assertIn("alt_opp", message)
            self.assertIn("--alt-boards", message)
            # Off, the old dataset still loads exactly as before.
            old = ppo.load_demo(path, bgpy.OBS_SIZE, augment=False)
            self.assertIsNone(old.alt_opp)

    def test_alternative_shape_is_checked(self):
        obs = blank_obs(rows=2)
        mask = torch.zeros((2, bgpy.N_ACTIONS), dtype=bool)
        action = torch.zeros(2, dtype=torch.long)
        split = np.asarray(["train", "heldout"])
        with self.assertRaises(SystemExit):
            ppo.DemoData(obs, mask, action, split,
                         alt_opp=torch.zeros((2, 3, PATCH - 1)))


if __name__ == "__main__":
    unittest.main()
