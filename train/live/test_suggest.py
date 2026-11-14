#!/usr/bin/env python3
"""Unit tests for the live Battlegrounds move suggester (train/live/suggest.py)."""

from __future__ import annotations

import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import unittest

os.environ.setdefault("OMP_NUM_THREADS", "2")
os.environ.setdefault("MKL_NUM_THREADS", "2")

LIVE_DIR = Path(__file__).resolve().parent
ROOT = LIVE_DIR.parents[1]
for _p in (str(ROOT), str(ROOT / "train"), str(LIVE_DIR)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

import numpy as np  # noqa: E402
import torch  # noqa: E402

torch.set_num_threads(2)

import bgpy  # noqa: E402
from model import masked  # noqa: E402
import ppo  # noqa: E402
import suggest  # noqa: E402


CKPT = ROOT / "train" / "runs" / "r1113-a1" / "best.pt"
SAMPLE_LOG = (
    Path.home()
    / "hearthstone-linux"
    / "hearthstone"
    / "Logs"
    / "Hearthstone_2026_11_14_00_45_13"
    / "Power.log"
)
# 1-indexed line slice for game 2 of SAMPLE_LOG, cut mid-turn 8 at step 7.
G2_START_LINE = 756568
G2_CUT_END_LINE = 855100


def _read_game2_slices(log_path: Path) -> tuple[list[str], list[str]]:
    """Return (full_game2_lines, cut_mid_turn8_lines) from SAMPLE_LOG."""
    cut: list[str] = []
    full: list[str] = []
    with log_path.open("r", encoding="utf-8", errors="replace") as fh:
        for lineno, line in enumerate(fh, start=1):
            if lineno < G2_START_LINE:
                continue
            if lineno > G2_START_LINE and "GameState.DebugPrintPower() - CREATE_GAME" in line:
                break
            full.append(line)
            if lineno <= G2_CUT_END_LINE:
                cut.append(line)
    return full, cut


def _bgbench_has_live_actions(bgbench_bin: Path) -> bool:
    proc = subprocess.run(
        [str(bgbench_bin), "live-actions"],
        cwd=str(ROOT),
        capture_output=True,
        text=True,
        check=False,
    )
    out = (proc.stderr + proc.stdout).lower()
    return "usage: bgbench live-actions" in out


def _collect_finished_actions(log_lines: list[str], out_dir: Path, name: str) -> Path:
    arch = out_dir / "arch" / name
    arch.mkdir(parents=True, exist_ok=True)
    (arch / "Power.log").write_text("".join(log_lines))
    out = out_dir / "actions"
    subprocess.run(
        [
            str(suggest.DEFAULT_BGBENCH),
            "collect-actions",
            "--out",
            str(out),
            "--cards",
            str(suggest.DEFAULT_HS_CARDS),
            "--archive",
            str(out_dir / "arch"),
            "--laptop",
            str(out_dir / "empty"),
            "--rewrite",
        ],
        cwd=str(ROOT),
        check=True,
        capture_output=True,
        text=True,
    )
    files = sorted(out.glob(f"{name}-g*.json"))
    if not files:
        raise RuntimeError("collect-actions produced no action file")
    return files[0]


def _era_turn_path_probs(
    action_file: Path,
    turn: int,
    target_step: int,
    ckpt_path: Path,
    top_k: int = 3,
) -> tuple[list[tuple[int, float]], int]:
    """Reference offline evaluation via EraTurnPath at (turn, target_step),
    with the same real lobby the live suggester seats."""
    scorer = suggest.PolicyScorer(ckpt_path, top_k=top_k)
    replay = bgpy.EraReplay.from_file(
        str(action_file), str(suggest.DEFAULT_REF_CARDS), lobby=True
    )
    path = replay.path(turn, 1, True)
    while not path.done and path.steps_on_path < target_step:
        a = path.era_action()
        assert a >= 0
        path.act(a)
    assert path.steps_on_path == target_step

    obs_np = np.zeros(bgpy.OBS_SIZE, dtype=np.float32)
    mask_np = np.zeros(bgpy.N_ACTIONS, dtype=bool)
    path.observe(obs_np, mask_np)

    scorer._apply_globals()
    o = torch.from_numpy(obs_np).unsqueeze(0)
    m = torch.from_numpy(mask_np).unsqueeze(0)
    fm = ppo.force_mask(o, m)
    with torch.no_grad():
        logits, _ = scorer.net(o, comp=None)
        probs = torch.softmax(masked(logits, fm), dim=-1)[0].cpu().numpy()
    legal = np.flatnonzero(fm[0].cpu().numpy())
    order = legal[np.argsort(-probs[legal], kind="stable")]
    return [(int(i), round(float(probs[i]), 4)) for i in order[:top_k]], path.steps_on_path


class TestSuggest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if not CKPT.is_file():
            raise unittest.SkipTest(f"missing test checkpoint: {CKPT}")
        cls.scorer = suggest.PolicyScorer(CKPT, top_k=3)

    def setUp(self) -> None:
        self.scorer.clear_history()

    def test_cut_log_matches_era_turn_path(self) -> None:
        if not SAMPLE_LOG.is_file() or not suggest.DEFAULT_BGBENCH.is_file():
            self.skipTest("sample Power.log or release bgbench binary unavailable")

        full_lines, cut_lines = _read_game2_slices(SAMPLE_LOG)
        self.assertGreater(len(cut_lines), 1000)

        with tempfile.TemporaryDirectory(prefix="bg_test_cut_") as tmp:
            tmp_dir = Path(tmp)
            sess = "Hearthstone_2026_11_14_00_45_13"
            full_json = _collect_finished_actions(full_lines, tmp_dir / "full", sess)
            cut_log = tmp_dir / sess / "Power.log"
            cut_log.parent.mkdir(parents=True, exist_ok=True)
            cut_log.write_text("".join(cut_lines))

            has_live = hasattr(bgpy.EraReplay, "from_file") and hasattr(bgpy.EraReplay, "live") and _bgbench_has_live_actions(suggest.DEFAULT_BGBENCH)
            if os.environ.get("BG_TEST_REQUIRE_LIVE") == "1":
                self.assertTrue(has_live, "expected live-actions and EraReplay.live to be active")
            # Run live suggester path on the cut-short copy.
            if has_live:
                decision = suggest.run_once(
                    cut_log,
                    self.scorer,
                    label=sess,
                    seed=1,
                )
            else:
                # Pre-rebuild fallback: cut slice already contains turn 8 step 7 after combat.
                cut_json = _collect_finished_actions(cut_lines, tmp_dir / "cut", sess)
                decision = suggest.handle_collector_event(
                    {
                        "event": "decision",
                        "game": f"{sess}-g1",
                        "file": str(cut_json),
                        "turn": 8,
                    },
                    self.scorer,
                    seed=1,
                )

            self.assertIsNotNone(decision)
            assert decision is not None
            self.assertEqual(decision["v"], 1)
            self.assertEqual(decision["event"], "decision")
            self.assertEqual(decision["turn"], 8)
            self.assertEqual(decision["step"], 7)
            self.assertIsNone(decision["value"])

            ref_top, ref_step = _era_turn_path_probs(
                full_json, turn=8, target_step=7, ckpt_path=CKPT, top_k=3
            )
            self.assertEqual(decision["step"], ref_step)
            self.assertEqual(len(decision["top"]), len(ref_top))

            # Compare both action order and probabilities against EraTurnPath.
            ref_labels = [
                self.scorer.last_decision.labels[idx]  # type: ignore[union-attr]
                for idx, _ in ref_top
            ]
            got_labels = [row["label"] for row in decision["top"]]
            got_probs = [row["p"] for row in decision["top"]]
            ref_probs = [p for _, p in ref_top]
            self.assertEqual(got_labels, ref_labels)
            for gp, rp in zip(got_probs, ref_probs):
                self.assertAlmostEqual(gp, rp, places=4)

    def test_v1_contract_fields_prev_and_top_k(self) -> None:
        class _FakePos:
            def __init__(self, turn: int, step: int, gold: int, tier: int, prev_action: int | None):
                self.turn = turn
                self.step = step
                self.gold = gold
                self.tier = tier
                self.prev_action = prev_action

            def card_names(self) -> list[str]:
                return ["Alleycat", "Southsea Busker", "Rockpool Hunter"]

            def observe(self, obs: np.ndarray, mask: np.ndarray) -> None:
                obs[bgpy.SCALAR_TURN] = float(self.turn)
                obs[bgpy.SCALAR_TIER] = float(self.tier)
                obs[bgpy.SCALAR_GOLD] = float(self.gold)
                # Populate 2 shop slots + roll + end turn.
                for s, cid in enumerate((1, 2)):
                    obs[bgpy.SHOP_START + s * bgpy.SLOT_FEATURES + bgpy.SLOT_CARD_ID] = float(cid)
                    obs[bgpy.SHOP_START + s * bgpy.SLOT_FEATURES + bgpy.SLOT_ATTACK] = 2.0
                    obs[bgpy.SHOP_START + s * bgpy.SLOT_FEATURES + bgpy.SLOT_HEALTH] = 2.0
                mask[0] = True
                mask[1] = True
                mask[suggest.action_layout()["roll"]] = True
                mask[bgpy.END_TURN] = True

        d0 = self.scorer.score_position(_FakePos(1, 0, 3, 1, None), game_id="sess-g1")
        for key in ("v", "event", "ts", "game", "turn", "step", "gold", "tier", "value", "top", "prev", "ckpt"):
            self.assertIn(key, d0)
        self.assertEqual(d0["v"], 1)
        self.assertEqual(d0["event"], "decision")
        self.assertIsInstance(d0["ts"], float)
        self.assertEqual(d0["game"], "sess-g1")
        self.assertIsNone(d0["value"])
        self.assertIsNone(d0["prev"])
        self.assertEqual(len(d0["top"]), 3)
        for row in d0["top"]:
            self.assertIn("kind", row)
            self.assertIn("label", row)
            self.assertIn("p", row)
            self.assertGreaterEqual(row["p"], 0.0)
            self.assertLessEqual(row["p"], 1.0)

        # Taking top pick at step 0 gives prev.rank == 1 at step 1.
        top0_idx = self.scorer.last_decision.top_indices[0]  # type: ignore[union-attr]
        d1 = self.scorer.score_position(_FakePos(1, 1, 0, 1, top0_idx), game_id="sess-g1")
        self.assertEqual(d1["prev"], {"label": d0["top"][0]["label"], "rank": 1})

        # Taking a legal move ranked outside --top 3 yields rank=None (unlisted).
        unlisted_idx = next(
            i for i in self.scorer.last_decision.labels  # type: ignore[union-attr]
            if i not in self.scorer.last_decision.top_indices  # type: ignore[union-attr]
        )
        d2 = self.scorer.score_position(_FakePos(1, 2, 0, 1, unlisted_idx), game_id="sess-g1")
        self.assertEqual(
            d2["prev"],
            {"label": self.scorer.last_decision.labels[unlisted_idx], "rank": None},  # type: ignore[union-attr]
        )

        # game_end clears prev history and matches v1 contract.
        ge = suggest.handle_collector_event(
            {"event": "game_end", "game": "sess-g1", "place": 2},
            self.scorer,
        )
        self.assertEqual(ge["v"], 1)
        self.assertEqual(ge["event"], "game_end")
        self.assertEqual(ge["game"], "sess-g1")
        self.assertEqual(ge["place"], 2)
        self.assertIsNone(self.scorer.last_decision)

        # Missing file triggers unsupported event cleanly instead of crashing.
        uns = suggest.handle_collector_event(
            {"event": "decision", "game": "sess-g2", "file": "/nonexistent/nope.json", "turn": 3},
            self.scorer,
        )
        self.assertEqual(uns["v"], 1)
        self.assertEqual(uns["event"], "unsupported")
        self.assertEqual(uns["game"], "sess-g2")
        self.assertEqual(uns["turn"], 3)
        self.assertTrue(uns["why"])

        # --top 1 returns exactly 1 move.
        scorer1 = suggest.PolicyScorer(CKPT, top_k=1)
        d_top1 = scorer1.score_position(_FakePos(1, 0, 3, 1, None), game_id="sess-g1")
        self.assertEqual(len(d_top1["top"]), 1)

    def test_socket_hello_latest_decision_and_broadcast(self) -> None:
        with tempfile.TemporaryDirectory(prefix="bg_test_sock_") as tmp:
            sock_path = Path(tmp) / "bg-suggest.sock"
            srv = suggest.SocketServer(sock_path, ckpt_name="r1113-a1/best.pt")
            try:
                d1 = {
                    "v": 1,
                    "event": "decision",
                    "ts": 1.0,
                    "game": "g-g1",
                    "turn": 3,
                    "step": 0,
                    "gold": 5,
                    "tier": 2,
                    "value": None,
                    "top": [{"kind": "roll", "label": "Roll", "p": 0.6}],
                    "prev": None,
                    "ckpt": "r1113-a1/best.pt",
                }
                srv.broadcast(d1)

                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as cli:
                    cli.settimeout(2.0)
                    cli.connect(str(sock_path))
                    f1 = cli.makefile("r", encoding="utf-8")
                    msg_hello = json.loads(f1.readline())
                    msg_d1 = json.loads(f1.readline())
                    self.assertEqual(msg_hello["event"], "hello")
                    self.assertEqual(msg_hello["v"], 1)
                    self.assertEqual(msg_hello["ckpt"], "r1113-a1/best.pt")
                    self.assertEqual(msg_d1, d1)

                    d2 = dict(d1, step=1, prev={"label": "Roll", "rank": 1})
                    srv.broadcast(d2)
                    msg_d2 = json.loads(f1.readline())
                    self.assertEqual(msg_d2, d2)

                    srv.broadcast({"v": 1, "event": "game_end", "ts": 2.0, "game": "g-g1", "place": 1})
                    msg_end = json.loads(f1.readline())
                    self.assertEqual(msg_end["event"], "game_end")

                # New client after game_end receives hello only (no stale decision).
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as cli2:
                    cli2.settimeout(0.3)
                    cli2.connect(str(sock_path))
                    f2 = cli2.makefile("r", encoding="utf-8")
                    msg_hello2 = json.loads(f2.readline())
                    self.assertEqual(msg_hello2["event"], "hello")
                    with self.assertRaises(TimeoutError):
                        f2.readline()
            finally:
                srv.close()
                self.assertFalse(sock_path.exists())

    def test_decode_all_action_kinds(self) -> None:
        offsets = suggest.action_layout()
        names = suggest.NameTables(
            cards=["Alleycat", "Coin"],
            heroes=["Nozdormu"],
            powers=["Clairvoyance"],
            trinkets=["Bob-blehead"],
        )
        obs = np.zeros(bgpy.OBS_SIZE, dtype=np.float32)
        obs[bgpy.SHOP_START + bgpy.SLOT_CARD_ID] = 1.0
        obs[bgpy.SHOP_SPELL_START + bgpy.SLOT_CARD_ID] = 2.0
        obs[bgpy.HAND_START + bgpy.SLOT_CARD_ID] = 1.0
        obs[bgpy.BOARD_START + bgpy.SLOT_CARD_ID] = 1.0
        obs[bgpy.DISCOVER_START + bgpy.SLOT_CARD_ID] = 1.0
        obs[bgpy.SCALAR_POWER_HERO] = 1.0
        obs[bgpy.SCALAR_POWER2_HERO] = 1.0
        obs[bgpy.SCALAR_TRINKET_OPTION] = 1.0
        obs[bgpy.SCALAR_TRINKET_OPTION + 1] = 3.0

        mask = np.ones(bgpy.N_ACTIONS, dtype=bool)
        for kind, _ in bgpy.ACTION_KINDS:
            idx = offsets[kind]
            out = suggest.decode_action(idx, obs, mask, names, offsets)
            self.assertIn("kind", out)
            self.assertTrue(out["label"])

    def test_find_newest_log(self) -> None:
        with tempfile.TemporaryDirectory(prefix="bg_test_logs_") as tmp:
            root = Path(tmp)
            self.assertIsNone(suggest.find_newest_log(root))
            old = root / "Hearthstone_2026_11_13_20_00_00" / "Power.log"
            new = root / "Hearthstone_2026_11_14_01_00_00" / "Power.log"
            old.parent.mkdir()
            new.parent.mkdir()
            old.write_text("a\n")
            new.write_text("b\n")
            self.assertEqual(suggest.find_newest_log(root), new)

    def test_resolve_paths_from_outside_cwd(self) -> None:
        old_cwd = os.getcwd()
        try:
            os.chdir("/tmp")
            p = suggest.resolve_path("train/runs/r1113-a1/best.pt")
            self.assertEqual(p, CKPT)
            self.assertEqual(suggest.ckpt_display_name(p), "r1113-a1/best.pt")
        finally:
            os.chdir(old_cwd)


if __name__ == "__main__":
    unittest.main()
