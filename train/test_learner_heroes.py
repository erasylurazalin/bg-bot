"""Tests for fixing the learner's hero (docs/briefs/learner-heroes.md).

    cd train && ../.venv/bin/python -m unittest test_learner_heroes

The ppo.py test runs one tiny CPU update in a temporary folder, so it needs
bgpy importable and torch. Set BG_BOT_PPO_SMOKE=0 to skip that slower check.
"""

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import bgpy
import torch

ROOT = Path(__file__).resolve().parent.parent
TRAIN = ROOT / "train"
HERO = "Trade Prince Gallywix"
LINEUP = [1, 0, 7, 0, 0, 0]


class ForcedHeroTest(unittest.TestCase):
    def test_gallywix_learner_for_200_games(self):
        env = bgpy.VecEnv(200, 20261113, LINEUP, learner_heroes=[HERO])
        for i in range(200):
            seats = env.seats(i)
            learners = [s for s in seats if s[0] == bgpy.LEARNER]
            self.assertEqual(len(learners), 1, f"game {i}: one learner")
            self.assertEqual(learners[0][4], HERO, f"game {i}: learner hero")
            self.assertEqual(sum(s[4] == HERO for s in seats), 1,
                             f"game {i}: only the learner is Gallywix")

    def test_empty_list_is_the_old_deal(self):
        plain = bgpy.VecEnv(8, 4242, LINEUP)
        empty = bgpy.VecEnv(8, 4242, LINEUP, learner_heroes=[])
        for i in range(8):
            self.assertEqual(plain.seats(i), empty.seats(i), f"game {i}")

    def test_unknown_name_is_refused_and_valid_ones_are_named(self):
        with self.assertRaises(ValueError) as caught:
            bgpy.VecEnv(1, 1, LINEUP, learner_heroes=["Bogus Hero"])
        msg = str(caught.exception)
        self.assertIn("Bogus Hero", msg)
        self.assertIn(HERO, msg)

    @unittest.skipIf(os.environ.get("BG_BOT_PPO_SMOKE") == "0",
                     "BG_BOT_PPO_SMOKE=0")
    def test_ppo_flag_reaches_the_env_and_the_checkpoint(self):
        name = "learner-heroes-test"
        env = os.environ.copy()
        env["CUDA_VISIBLE_DEVICES"] = ""
        env["PYTHONPATH"] = os.pathsep.join(
            [p for p in sys.path if p] + [str(TRAIN)])
        cmd = [
            sys.executable, str(TRAIN / "ppo.py"),
            "--name", name, "--minutes", "0", "--seed", "1",
            "--envs", "1", "--steps", "1", "--eval-games", "1",
            "--eval-every", "1", "--lineup", "1,0,7,0,0,0",
            "--old-boards", "keep", "--learner-heroes", HERO,
        ]
        with tempfile.TemporaryDirectory() as td:
            done = subprocess.run(cmd, cwd=td, env=env, capture_output=True,
                                  text=True, timeout=300)
            self.assertEqual(done.returncode, 0,
                             msg=(done.stdout or "") + (done.stderr or ""))
            self.assertIn(f"learner heroes: {HERO}", done.stdout)
            ck = torch.load(Path(td) / "runs" / name / "ckpt.pt",
                            map_location="cpu", weights_only=False)
            self.assertEqual(ck["cfg"]["learner_heroes"], [HERO])


if __name__ == "__main__":
    unittest.main()
