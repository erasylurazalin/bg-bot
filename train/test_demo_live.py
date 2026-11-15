"""Tests for the live-lobby demonstration retry (CPU only).

    cd train && ../.venv/bin/python -m unittest test_demo_live

The Rust lobby seating is tested in `cargo test -p bgpy`; this file checks the
Python defaults, the dataset's alive check on a small fixture, the
demonstration batch draw, and the update stop condition.
"""

import json
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np
import torch

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "train" / "evaltools"))

import bgpy
import ppo
from test_demo_loss import fixture as demo_fixture

FIXTURE = ROOT / "crates" / "bgcore" / "tests" / "data" / "actions" / "fixture-g2.json"
REF_CARDS = ROOT / "ref" / "bg-cards.json"

LOBBIES = [
    {"player_id": 1, "seat": 1, "health": 30, "armor": 0, "tier": 1,
     "alive": True, "placement": 0},
    {"player_id": 2, "seat": 2, "health": 27, "armor": 2, "tier": 2,
     "alive": True, "placement": 0},
    {"player_id": 3, "seat": 3, "health": 0, "armor": 0, "tier": 2,
     "alive": False, "placement": 7},
    {"player_id": 4, "seat": 4, "health": 20, "armor": 0, "tier": 3,
     "alive": True, "placement": 0},
    {"player_id": 5, "seat": 5, "health": 18, "armor": 1, "tier": 3,
     "alive": True, "placement": 0},
    {"player_id": 6, "seat": 6, "health": 14, "armor": 0, "tier": 4,
     "alive": True, "placement": 0},
    {"player_id": 7, "seat": 7, "health": 9, "armor": 0, "tier": 4,
     "alive": True, "placement": 0},
]


def fixture_action_file(directory: Path) -> Path:
    """The bgcore action fixture with a seven-seat lobby block added."""
    text = FIXTURE.read_text().replace(
        '"start": {', '"start": {"lobby": ' + json.dumps(LOBBIES) + ",")
    path = directory / "fixture-g2.json"
    path.write_text(text)
    return path


def observation(replay, turn):
    path = replay.path(turn, 1, True)
    obs = np.zeros(bgpy.OBS_SIZE, dtype=np.float32)
    mask = np.zeros(bgpy.N_ACTIONS, dtype=bool)
    path.observe(obs, mask)
    return path, obs


class LiveLobbyTest(unittest.TestCase):
    def test_from_file_seats_the_lobby_by_default_and_can_opt_out(self):
        with tempfile.TemporaryDirectory() as td:
            path = fixture_action_file(Path(td))
            on = bgpy.EraReplay.from_file(str(path), str(REF_CARDS))
            off = bgpy.EraReplay.from_file(str(path), str(REF_CARDS), lobby=False)
            turn = on.all_turns()[0]
            _path, seen = observation(on, turn)
            _path, dead = observation(off, turn)
            self.assertEqual(int(seen[bgpy.SCALAR_ALIVE]), 7)
            self.assertEqual(int(dead[bgpy.SCALAR_ALIVE]), 1)
            for seat in range(1, 8):
                at = bgpy.OWN_END + (seat - 1) * bgpy.OPP_BLOCK
                entry = LOBBIES[seat - 1]
                self.assertEqual(int(seen[at + bgpy.OPP_ALIVE]),
                                 int(entry["alive"]))
                self.assertEqual(int(seen[at + bgpy.OPP_HEALTH]), entry["health"])
                self.assertEqual(int(seen[at + bgpy.OPP_TIER]), entry["tier"])
                self.assertEqual(int(dead[at + bgpy.OPP_ALIVE]), 0)
                self.assertEqual(int(dead[at + bgpy.OPP_HEALTH]), 30)

    def test_alive_check_accepts_a_matching_lobby_fixture(self):
        sys.path.insert(0, str(ROOT / "train" / "evaltools"))
        import demo_dataset as dd

        with tempfile.TemporaryDirectory() as td:
            path = fixture_action_file(Path(td))
            replay = bgpy.EraReplay.from_file(str(path), str(REF_CARDS))
            turn = replay.all_turns()[0]
            _path, obs = observation(replay, turn)
            old = dd.ACTIONS
            dd.ACTIONS = Path(td)
            try:
                arrays = {
                    "obs": obs[None, :],
                    "game": np.asarray([path.name]),
                    "turn": np.asarray([turn]),
                }
                got = dd.alive_check(arrays, [path.name])
            finally:
                dd.ACTIONS = old
            self.assertEqual(got["rows"], 1)
            self.assertEqual(got["games"], 1)
            self.assertEqual(got["problems"], [])

    def test_alive_check_reports_a_changed_health(self):
        sys.path.insert(0, str(ROOT / "train" / "evaltools"))
        import demo_dataset as dd

        with tempfile.TemporaryDirectory() as td:
            path = fixture_action_file(Path(td))
            replay = bgpy.EraReplay.from_file(str(path), str(REF_CARDS))
            turn = replay.all_turns()[0]
            _path, obs = observation(replay, turn)
            obs = obs.copy()
            obs[bgpy.OWN_END + bgpy.OPP_HEALTH] += 1
            old = dd.ACTIONS
            dd.ACTIONS = Path(td)
            try:
                arrays = {
                    "obs": obs[None, :],
                    "game": np.asarray([path.name]),
                    "turn": np.asarray([turn]),
                }
                got = dd.alive_check(arrays, [path.name])
            finally:
                dd.ACTIONS = old
            self.assertEqual(len(got["problems"]), 1)
            self.assertEqual(got["problems"][0]["field"], "OPP_HEALTH")


class DemoBatchTest(unittest.TestCase):
    def test_demo_batch_changes_the_rows_drawn_and_nothing_else(self):
        obs, mask, action, split = demo_fixture()
        demo = ppo.DemoData(obs, mask, action, split)
        m = 8
        old = demo.train[torch.randint(
            len(demo.train), (m,), generator=torch.Generator().manual_seed(3))]
        now = ppo.demo_rows_for_minibatch(
            demo, m, torch.Generator().manual_seed(3), 0)
        self.assertTrue(torch.equal(old, now),
                        "demo-batch 0 must draw exactly the old rows")
        small = ppo.demo_rows_for_minibatch(
            demo, m, torch.Generator().manual_seed(3), 3)
        self.assertEqual(len(small), 3)
        self.assertTrue(torch.equal(small, now[:3]))

        pieces = [ppo.demo_piece(small, j, 2, m) for j in range(0, m, 2)]
        used = torch.cat(pieces)
        self.assertEqual(sorted(used.tolist()), sorted(small.tolist()))
        self.assertEqual(len(used), len(small))


class MaxUpdatesTest(unittest.TestCase):
    def test_update_cap_stops_at_n_or_the_minute_limit(self):
        self.assertFalse(ppo.training_done(1.0, 10.0, 4, 5))
        self.assertTrue(ppo.training_done(1.0, 10.0, 5, 5))
        self.assertFalse(ppo.training_done(9.9, 10.0, 100, 0))
        self.assertTrue(ppo.training_done(10.0, 10.0, 3, 100))
        self.assertFalse(ppo.training_done(0.0, 10.0, 100, 0))


if __name__ == "__main__":
    unittest.main()
