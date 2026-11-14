#!/usr/bin/env python3
"""Live Battlegrounds move suggester for bg-overlay.

Follows Hearthstone's newest Power.log via ``bgbench live-actions``, seats the
current shop position with ``bgcore::shop_replay::seat_live`` through ``bgpy``,
runs a trained policy checkpoint on CPU, and broadcasts v1 JSON lines over a
Unix stream socket ($XDG_RUNTIME_DIR/bg-suggest.sock by default).

The suggester never sends input to Hearthstone or touches any game files
besides reading Power.log.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
from typing import Any

# Cap CPU threads before importing torch so inference never competes with the game.
os.environ.setdefault("OMP_NUM_THREADS", "2")
os.environ.setdefault("MKL_NUM_THREADS", "2")

ROOT = Path(__file__).resolve().parents[2]
TRAIN_DIR = ROOT / "train"
for _p in (str(ROOT), str(TRAIN_DIR)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

import numpy as np  # noqa: E402
import torch  # noqa: E402

import bgpy  # noqa: E402
import fixed_eval as fe  # noqa: E402
from model import masked  # noqa: E402
import ppo  # noqa: E402


DEFAULT_LOGS = Path.home() / "hearthstone-linux" / "hearthstone" / "Logs"
DEFAULT_REF_CARDS = ROOT / "ref" / "bg-cards.json"
DEFAULT_HS_CARDS = ROOT / "ref" / "hsjson-cards.json"
DEFAULT_BGBENCH = ROOT / "target" / "release" / "bgbench"


class UnsupportedPosition(Exception):
    """Raised when a live position cannot be seated or contains unbuilt cards."""


def default_socket() -> Path:
    runtime = os.environ.get("XDG_RUNTIME_DIR") or "/tmp"
    return Path(runtime) / "bg-suggest.sock"


def resolve_path(value: str | Path, must_exist: bool = True) -> Path:
    """Resolve a user path from CWD first, falling back to repo ROOT."""
    p = Path(value).expanduser()
    if not p.is_absolute() and not p.exists():
        cand = ROOT / p
        if cand.exists() or not must_exist:
            return cand
    return p


def ckpt_display_name(ckpt: Path) -> str:
    if ckpt.name in ("best.pt", "last.pt") and ckpt.parent.name:
        return f"{ckpt.parent.name}/{ckpt.name}"
    return ckpt.name


def action_layout() -> dict[str, int]:
    """Compute flat action offsets directly from bgpy.ACTION_KINDS."""
    offsets: dict[str, int] = {}
    pos = 0
    for kind, count in bgpy.ACTION_KINDS:
        offsets[kind] = pos
        pos += count
    return offsets


def _load_toml_names() -> tuple[list[str], list[str], list[str]]:
    """Fallback name tables when bgpy lacks LivePosition getters."""
    import re

    t_text = (ROOT / "trinkets.toml").read_text()
    t_blocks = re.split(r"(?m)^\[\[[^\]]+\]\]\s*$", t_text)[1:]
    trinkets = []
    for block in t_blocks:
        m_name = re.search(r'(?m)^name = "([^"]+)"', block)
        m_card = re.search(r'(?m)^card_id = "([^"]+)"', block)
        if m_name and m_card:
            trinkets.append(m_name.group(1))
    hero_doc = tomllib.loads((ROOT / "heroes.toml").read_text())
    heroes = [h["name"] for h in hero_doc.get("hero", [])]
    powers = [h["power"]["name"] for h in hero_doc.get("hero", []) if "power" in h]
    powers.extend(p["name"] for p in hero_doc.get("power", []))
    return heroes, powers, trinkets


@dataclass
class NameTables:
    cards: list[str]
    heroes: list[str]
    powers: list[str]
    trinkets: list[str]

    @classmethod
    def from_pos(cls, pos: Any) -> "NameTables":
        cards = list(pos.card_names())
        if hasattr(pos, "power_names") and hasattr(pos, "hero_names") and hasattr(pos, "trinket_names"):
            return cls(
                cards=cards,
                heroes=list(pos.hero_names()),
                powers=list(pos.power_names()),
                trinkets=list(pos.trinket_names()),
            )
        heroes, powers, trinkets = _load_toml_names()
        return cls(cards=cards, heroes=heroes, powers=powers, trinkets=trinkets)


def _slot_card(obs: np.ndarray, start: int, slot: int, cards: list[str]) -> str | None:
    cid = int(obs[start + slot * bgpy.SLOT_FEATURES + bgpy.SLOT_CARD_ID])
    if 1 <= cid <= len(cards):
        return cards[cid - 1]
    return None


def _indexed_name(value: float, table: list[str]) -> str | None:
    idx = int(value) - 1
    if 0 <= idx < len(table):
        return table[idx]
    return None


def _decode_choose(idx: int, obs: np.ndarray, names: NameTables) -> dict[str, Any]:
    if obs[bgpy.SCALAR_PENDING_POWER] > 0.5:
        pname = _indexed_name(obs[bgpy.SCALAR_POWER_OFFER + idx], names.powers)
        return {
            "kind": "discover",
            "slot": idx + 1,
            "label": f"Pick {pname}" if pname else f"Pick power {idx + 1}",
        }
    if obs[bgpy.SCALAR_PENDING_HERO] > 0.5:
        hname = _indexed_name(obs[bgpy.SCALAR_HERO_OFFER + idx], names.heroes)
        return {
            "kind": "discover",
            "slot": idx + 1,
            "label": f"Hero: {hname}" if hname else f"Hero {idx + 1}",
        }
    if obs[bgpy.SCALAR_PENDING_HOLMES] > 0.5:
        cname = _indexed_name(obs[bgpy.SCALAR_HOLMES_OFFER + idx], names.cards)
        item: dict[str, Any] = {
            "kind": "discover",
            "slot": idx + 1,
            "label": f"Guess {cname}" if cname else f"Guess {idx + 1}",
        }
        if cname:
            item["card"] = cname
        return item
    if obs[bgpy.SCALAR_PENDING_WAGER] > 0.5:
        return {"kind": "discover", "slot": idx + 1, "label": f"Wager {idx + 1}"}
    cname = _slot_card(obs, bgpy.DISCOVER_START, idx, names.cards)
    if cname:
        return {"kind": "discover", "slot": idx + 1, "card": cname, "label": f"Pick {cname}"}
    return {"kind": "discover", "slot": idx + 1, "label": f"Choose option {idx + 1}"}


def decode_action(
    action_idx: int,
    obs: np.ndarray,
    mask: np.ndarray,
    names: NameTables,
    offsets: dict[str, int],
) -> dict[str, Any]:
    """Decode one flat engine action index into a contract-v1 move entry."""
    shop = bgpy.SHOP
    board = bgpy.BOARD
    hand = bgpy.HAND
    move_stride = board - 1
    play_stride = board + 1

    kind_name = ""
    local = 0
    for k, count in bgpy.ACTION_KINDS:
        start = offsets[k]
        if start <= action_idx < start + count:
            kind_name = k
            local = action_idx - start
            break

    if kind_name == "buy":
        card = _slot_card(obs, bgpy.SHOP_START, local, names.cards)
        out: dict[str, Any] = {
            "kind": "buy",
            "slot": local + 1,
            "label": f"Buy {card}" if card else f"Buy slot {local + 1}",
        }
        if card:
            out["card"] = card
        return out

    if kind_name == "buy_spell":
        card = _slot_card(obs, bgpy.SHOP_SPELL_START, 0, names.cards)
        out = {"kind": "buy", "label": f"Buy {card}" if card else "Buy Tavern spell"}
        if card:
            out["card"] = card
        return out

    if kind_name == "sell":
        card = _slot_card(obs, bgpy.BOARD_START, local, names.cards)
        out = {
            "kind": "sell",
            "slot": local + 1,
            "label": f"Sell {card}" if card else f"Sell slot {local + 1}",
        }
        if card:
            out["card"] = card
        return out

    if kind_name == "activate":
        card = _slot_card(obs, bgpy.BOARD_START, local, names.cards)
        out = {
            "kind": "play",
            "slot": local + 1,
            "label": f"Activate {card}" if card else f"Activate slot {local + 1}",
        }
        if card:
            out["card"] = card
        return out

    if kind_name == "play":
        h_idx = local // play_stride
        pos = local % play_stride
        card = _slot_card(obs, bgpy.HAND_START, h_idx, names.cards)
        base = offsets["play"] + h_idx * play_stride
        legal_positions = int(mask[base : base + play_stride].sum())
        name = card or f"hand {h_idx + 1}"
        if legal_positions > 1:
            label = f"Play {name} -> {pos + 1}"
        else:
            label = f"Play {name}"
        out = {"kind": "play", "slot": h_idx + 1, "label": label}
        if card:
            out["card"] = card
        return out

    if kind_name == "target":
        card = _slot_card(obs, bgpy.BOARD_START, local, names.cards)
        out = {
            "kind": "discover",
            "slot": local + 1,
            "label": f"Target {card}" if card else f"Target slot {local + 1}",
        }
        if card:
            out["card"] = card
        return out

    if kind_name == "target_shop":
        card = _slot_card(obs, bgpy.SHOP_START, local, names.cards)
        out = {
            "kind": "discover",
            "slot": local + 1,
            "label": f"Target {card}" if card else f"Target Tavern {local + 1}",
        }
        if card:
            out["card"] = card
        return out

    if kind_name == "choose":
        return _decode_choose(local, obs, names)

    if kind_name == "discard":
        card = _slot_card(obs, bgpy.HAND_START, local, names.cards)
        out = {
            "kind": "discover",
            "slot": local + 1,
            "label": f"Discard {card}" if card else f"Discard hand {local + 1}",
        }
        if card:
            out["card"] = card
        return out

    if kind_name == "move":
        from_s = local // move_stride
        raw_to = local % move_stride
        to_s = raw_to + 1 if raw_to >= from_s else raw_to
        card = _slot_card(obs, bgpy.BOARD_START, from_s, names.cards)
        prefix = f"Move {card}" if card else "Move minion"
        out = {
            "kind": "move",
            "slot": from_s + 1,
            "label": f"{prefix} {from_s + 1} -> {to_s + 1}",
        }
        if card:
            out["card"] = card
        return out

    if kind_name == "roll":
        return {"kind": "roll", "label": "Roll"}
    if kind_name == "freeze":
        return {"kind": "freeze", "label": "Freeze"}
    if kind_name == "level":
        return {"kind": "level", "label": "Tier up"}
    if kind_name == "end":
        return {"kind": "end", "label": "End turn"}

    if kind_name == "hero_power":
        pname = _indexed_name(obs[bgpy.SCALAR_POWER_HERO], names.powers)
        label = f"Hero power: {pname}" if pname else "Hero power"
        return {"kind": "hero_power", "label": label}

    if kind_name == "hero_power2":
        pname = _indexed_name(obs[bgpy.SCALAR_POWER2_HERO], names.powers)
        label = f"Hero power 2: {pname}" if pname else "Hero power 2"
        return {"kind": "hero_power", "label": label}

    if kind_name == "choose_trinket":
        base = bgpy.SCALAR_TRINKET_OPTION + local * bgpy.TRINKET_OPTION_FEATURES
        tname = _indexed_name(obs[base], names.trinkets)
        price = int(obs[base + 1])
        label = f"Pick {tname} ({price}g)" if tname else f"Pick trinket {local + 1}"
        return {"kind": "discover", "slot": local + 1, "label": label}

    if kind_name == "dark_gift":
        return {"kind": "hero_power", "label": "Dark Discovery"}

    # Unreachable for valid bgpy.N_ACTIONS, kept defensive against shape changes.
    _ = shop, hand
    return {"kind": kind_name or "other", "label": kind_name or f"Action {action_idx}"}


@dataclass
class PriorDecision:
    game: str
    turn: int
    step: int
    top_indices: list[int]
    labels: dict[int, str]


class PolicyScorer:
    """CPU policy scorer matching fixed_eval / demo_dataset masking."""

    def __init__(self, ckpt_path: Path, top_k: int = 3):
        torch.set_num_threads(2)
        self.ckpt_path = resolve_path(ckpt_path)
        self.ckpt_name = ckpt_display_name(self.ckpt_path)
        self.top_k = max(1, int(top_k))
        self.dev = torch.device("cpu")
        info = fe.load_ckpt(str(self.ckpt_path), self.dev)
        if info["n_comps"]:
            raise SystemExit(
                f"{self.ckpt_path}: checkpoint expects {info['n_comps']} comp inputs, "
                "which live suggestion does not feed"
            )
        ck = torch.load(str(self.ckpt_path), map_location="cpu", weights_only=False)
        cfg = ck.get("cfg") or {}
        if cfg.get("lag_in_obs"):
            raise SystemExit(f"{self.ckpt_path}: checkpoint trained with lag_in_obs, unsupported live")
        self.net = info["net"]
        self.force_dark = bool(info["force_dark_gift"])
        self.old_boards = info["old_boards"]
        self.offsets = action_layout()
        self.names: NameTables | None = None
        self.last_decision: PriorDecision | None = None

    def _apply_globals(self) -> None:
        ppo.FORCE_DARK = self.force_dark
        ppo.FORCE_TURN2 = False
        ppo.OLD_BOARDS = self.old_boards

    def clear_history(self) -> None:
        self.last_decision = None

    def score_position(
        self,
        pos: Any,
        game_id: str,
        prev_fallback_label: str | None = None,
    ) -> dict[str, Any]:
        """Compute a v1 decision event dict for a seated LivePosition or EraTurnPath."""
        if self.names is None:
            self.names = NameTables.from_pos(pos)

        obs_np = np.zeros(bgpy.OBS_SIZE, dtype=np.float32)
        mask_np = np.zeros(bgpy.N_ACTIONS, dtype=bool)
        pos.observe(obs_np, mask_np)

        if not mask_np.any():
            raise UnsupportedPosition("no legal actions in this position")

        self._apply_globals()
        o = torch.from_numpy(obs_np).unsqueeze(0)
        m = torch.from_numpy(mask_np).unsqueeze(0)
        fm = ppo.force_mask(o, m)
        if not fm.any():
            fm = m

        with torch.no_grad():
            logits, _ = self.net(o, comp=None)
            probs = torch.softmax(masked(logits, fm), dim=-1)[0].cpu().numpy()

        fm_np = fm[0].cpu().numpy()
        legal = np.flatnonzero(fm_np)
        order = legal[np.argsort(-probs[legal], kind="stable")]

        labels: dict[int, str] = {}
        entries: dict[int, dict[str, Any]] = {}
        for idx_np in order:
            idx = int(idx_np)
            entry = decode_action(idx, obs_np, fm_np, self.names, self.offsets)
            labels[idx] = entry["label"]
            entries[idx] = entry

        top_indices = [int(i) for i in order[: self.top_k]]
        top = [{**entries[idx], "p": round(float(probs[idx]), 4)} for idx in top_indices]

        turn = int(getattr(pos, "turn"))
        step = int(getattr(pos, "step"))
        raw_gold = getattr(pos, "gold", None)
        raw_tier = getattr(pos, "tier", None)
        gold = int(round(float(obs_np[bgpy.SCALAR_GOLD]))) if raw_gold is None else int(raw_gold)
        tier = int(round(float(obs_np[bgpy.SCALAR_TIER]))) if raw_tier is None else int(raw_tier)
        prev_action = getattr(pos, "prev_action", None)

        prev: dict[str, Any] | None = None
        prior = self.last_decision
        if (
            step > 0
            and prior is not None
            and prior.game == game_id
            and prior.turn == turn
            and prior.step == step - 1
        ):
            rank: int | None = None
            if prev_action is not None and int(prev_action) in prior.top_indices:
                rank = prior.top_indices.index(int(prev_action)) + 1
            if prev_action is not None and int(prev_action) in prior.labels:
                plabel = prior.labels[int(prev_action)]
            elif prev_fallback_label:
                plabel = prev_fallback_label
            else:
                plabel = "Previous move"
            prev = {"label": plabel, "rank": rank}

        self.last_decision = PriorDecision(
            game=game_id,
            turn=turn,
            step=step,
            top_indices=top_indices,
            labels=labels,
        )

        # Value head outputs turn-discounted sandbox return rather than real 1..8 placement.
        return {
            "v": 1,
            "event": "decision",
            "ts": round(time.time(), 3),
            "game": game_id,
            "turn": turn,
            "step": step,
            "gold": gold,
            "tier": tier,
            "value": None,
            "top": top,
            "prev": prev,
            "ckpt": self.ckpt_name,
        }


def _describe_logged_step(step_spec: dict[str, Any]) -> str:
    kind = step_spec.get("step", "move")
    card = (step_spec.get("card") or {}).get("name") or (step_spec.get("target") or {}).get("name")
    picked = (step_spec.get("picked") or {}).get("name")
    verbs = {
        "buy": "Buy",
        "sell": "Sell",
        "play": "Play",
        "activate": "Activate",
        "discover": "Pick",
        "trinket_pick": "Pick",
        "hero_power": "Hero power",
        "roll": "Roll",
        "freeze": "Freeze",
        "level": "Tier up",
        "end_turn": "End turn",
        "dark_discovery": "Dark Discovery",
    }
    verb = verbs.get(kind, kind.replace("_", " ").capitalize())
    if picked:
        return f"{verb} {picked}"
    if card and kind not in ("roll", "freeze", "level", "end_turn", "dark_discovery"):
        return f"{verb} {card}"
    return verb


def seat_action_file(
    action_json_path: Path,
    ref_cards: Path = DEFAULT_REF_CARDS,
    turn: int | None = None,
    seed: int = 1,
) -> tuple[Any, str | None]:
    """Seat the latest live position from an action JSON file.

    Uses ``bgpy.EraReplay.from_file().live()`` when compiled in, falling back to
    ``EraTurnPath`` with a trailing ``end_turn`` for pre-rebuild wheels.
    """
    raw = json.loads(action_json_path.read_text())
    turns = raw.get("turns") or []
    if not turns:
        raise UnsupportedPosition("no shop turn recorded yet")
    target_turn_spec = (
        next((t for t in turns if t.get("turn") == turn), None) if turn is not None else turns[-1]
    )
    if target_turn_spec is None:
        raise UnsupportedPosition(f"turn {turn} not in action file")

    prev_label: str | None = None
    actions = target_turn_spec.get("actions") or []
    if actions:
        prev_label = _describe_logged_step(actions[-1])

    if hasattr(bgpy.EraReplay, "from_file") and hasattr(bgpy.EraReplay, "live"):
        replay = bgpy.EraReplay.from_file(str(action_json_path), str(ref_cards))
        try:
            pos = replay.live(turn=target_turn_spec["turn"], seed=seed)
        except ValueError as err:
            raise UnsupportedPosition(str(err).splitlines()[0]) from err
        blocking = [m for m in pos.missing if "not built" in m or "not name" in m]
        if blocking:
            raise UnsupportedPosition(blocking[0])
        return pos, prev_label

    # Fallback when running against a pre-rebuild bgpy wheel in .venv.
    if target_turn_spec.get("pending_choice") is not None:
        raise UnsupportedPosition("open choice requires rebuilt bgpy (EraReplay.live)")

    patched = dict(raw)
    patched_turns = []
    target_num = target_turn_spec["turn"]
    for t in turns:
        if t.get("turn") == target_num:
            t_copy = dict(t)
            t_copy["end"] = None
            t_copy["actions"] = list(actions) + [
                {"step": "end_turn", "paid": {"gold": 0, "health": 0}}
            ]
            patched_turns.append(t_copy)
        else:
            patched_turns.append(t)
    patched["turns"] = patched_turns

    with tempfile.TemporaryDirectory(prefix="bg_live_seat_") as tmp:
        (Path(tmp) / action_json_path.name).write_text(json.dumps(patched))
        replays, bad = bgpy.era_replays(tmp, str(ref_cards))
        if bad or not replays:
            why = bad[0] if bad else "failed to parse live action file"
            raise UnsupportedPosition(why)
        replay = replays[0]
        try:
            path = replay.path(target_num, seed, True)
        except ValueError as err:
            raise UnsupportedPosition(str(err).splitlines()[0]) from err

    k = len(actions)
    last_era: int | None = None
    while not path.done and path.steps_on_path < k:
        a = path.era_action()
        if a < 0 or path.steps_on_path >= k:
            break
        last_era = a
        path.act(a)

    if path.steps_on_path != k:
        raise UnsupportedPosition(
            f"could not reach live step {k} (stopped at {path.steps_on_path})"
        )
    blocking = [m for m in path.missing if "not built" in m or "not name" in m]
    if blocking:
        raise UnsupportedPosition(blocking[0])

    class _PathPosition:
        def __init__(self) -> None:
            self._path = path
            self.turn = int(target_num)
            self.step = int(path.steps_on_path)
            self.gold = None
            self.tier = None
            self.prev_action = last_era

        def observe(self, obs: np.ndarray, mask: np.ndarray) -> None:
            self._path.observe(obs, mask)

        def card_names(self) -> list[str]:
            return list(self._path.card_names())

    return _PathPosition(), prev_label


def find_newest_log(logs_dir: Path) -> Path | None:
    """Return the newest Power.log under logs_dir/Hearthstone_*/Power.log (or logs_dir/Power.log)."""
    if not logs_dir.exists():
        return None
    if logs_dir.is_file() and logs_dir.name == "Power.log":
        return logs_dir
    direct = logs_dir / "Power.log"
    candidates: list[Path] = []
    for entry in logs_dir.iterdir():
        if entry.is_dir() and entry.name.startswith("Hearthstone_"):
            p = entry / "Power.log"
            if p.is_file():
                candidates.append(p)
    if candidates:
        candidates.sort(key=lambda p: (p.parent.name, p.stat().st_mtime))
        return candidates[-1]
    if direct.is_file():
        return direct
    return None


class SocketServer:
    """Thread-safe multi-client Unix stream socket broadcaster matching contract v1."""

    def __init__(self, sock_path: Path, ckpt_name: str):
        self.sock_path = sock_path
        self.ckpt_name = ckpt_name
        self._lock = threading.Lock()
        self._clients: list[socket.socket] = []
        self._latest_turn_decision: dict[str, Any] | None = None
        self._stop = threading.Event()

        self.sock_path.parent.mkdir(parents=True, exist_ok=True)
        self.sock_path.unlink(missing_ok=True)
        self._srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._srv.bind(str(self.sock_path))
        self._srv.listen(8)
        self._srv.settimeout(0.25)
        self._thread = threading.Thread(target=self._accept_loop, daemon=True)
        self._thread.start()

    @staticmethod
    def _encode(payload: dict[str, Any]) -> bytes:
        return (json.dumps(payload, separators=(",", ":")) + "\n").encode("utf-8")

    def _send_one(self, conn: socket.socket, data: bytes) -> bool:
        try:
            conn.sendall(data)
            return True
        except OSError:
            return False

    def _accept_loop(self) -> None:
        while not self._stop.is_set():
            try:
                conn, _ = self._srv.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            conn.setblocking(True)
            hello = self._encode({
                "v": 1,
                "event": "hello",
                "ts": round(time.time(), 3),
                "ckpt": self.ckpt_name,
            })
            with self._lock:
                ok = self._send_one(conn, hello)
                if ok and self._latest_turn_decision is not None:
                    ok = self._send_one(conn, self._encode(self._latest_turn_decision))
                if ok:
                    self._clients.append(conn)
                else:
                    try:
                        conn.close()
                    except OSError:
                        pass

    def broadcast(self, payload: dict[str, Any]) -> None:
        raw = self._encode(payload)
        event = payload.get("event")
        with self._lock:
            if event == "decision":
                self._latest_turn_decision = payload
            elif event == "game_end":
                self._latest_turn_decision = None
            elif event == "unsupported":
                latest = self._latest_turn_decision
                if latest is not None and (
                    latest.get("game") != payload.get("game")
                    or latest.get("turn") != payload.get("turn")
                ):
                    self._latest_turn_decision = None
            alive: list[socket.socket] = []
            for conn in self._clients:
                if self._send_one(conn, raw):
                    alive.append(conn)
                else:
                    try:
                        conn.close()
                    except OSError:
                        pass
            self._clients = alive

    def close(self) -> None:
        self._stop.set()
        try:
            self._srv.close()
        except OSError:
            pass
        self._thread.join(timeout=1.0)
        with self._lock:
            for conn in self._clients:
                try:
                    conn.close()
                except OSError:
                    pass
            self._clients.clear()
        self.sock_path.unlink(missing_ok=True)


def run_once(
    log_path: Path,
    scorer: PolicyScorer,
    bgbench_bin: Path = DEFAULT_BGBENCH,
    ref_cards: Path = DEFAULT_REF_CARDS,
    hs_cards: Path = DEFAULT_HS_CARDS,
    label: str = "",
    seed: int = 1,
) -> dict[str, Any] | None:
    """Evaluate the latest decision in a Power.log snapshot using bgbench live-actions."""
    with tempfile.TemporaryDirectory(prefix="bg_live_once_") as tmp:
        cmd = [
            str(bgbench_bin),
            "live-actions",
            "--live-once",
            str(log_path),
            "--out",
            tmp,
            "--cards",
            str(hs_cards),
        ]
        if label:
            cmd.extend(["--label", label])
        proc = subprocess.run(cmd, cwd=str(ROOT), capture_output=True, text=True, check=False)
        if proc.returncode != 0:
            raise RuntimeError(proc.stderr.strip() or f"bgbench exited with {proc.returncode}")
        lines = [ln for ln in proc.stdout.splitlines() if ln.strip()]
        if not lines:
            return None
        ev = json.loads(lines[-1])
        return handle_collector_event(ev, scorer, ref_cards=ref_cards, seed=seed)


def handle_collector_event(
    ev: dict[str, Any],
    scorer: PolicyScorer,
    ref_cards: Path = DEFAULT_REF_CARDS,
    seed: int = 1,
) -> dict[str, Any]:
    """Turn a bgbench collector event into a v1 socket payload."""
    kind = ev.get("event")
    game_id = ev.get("game", "")
    if kind == "game_end":
        scorer.clear_history()
        return {
            "v": 1,
            "event": "game_end",
            "ts": round(time.time(), 3),
            "game": game_id,
            "place": ev.get("place"),
        }
    if kind == "decision":
        turn = ev.get("turn")
        try:
            pos, prev_label = seat_action_file(
                Path(ev["file"]),
                ref_cards=ref_cards,
                turn=turn,
                seed=seed,
            )
            return scorer.score_position(pos, game_id=game_id, prev_fallback_label=prev_label)
        except UnsupportedPosition as err:
            return {
                "v": 1,
                "event": "unsupported",
                "ts": round(time.time(), 3),
                "game": game_id,
                "turn": int(turn or 0),
                "why": str(err)[:160],
            }
        except Exception as err:  # keep live suggester alive on unexpected parser/engine edge cases
            return {
                "v": 1,
                "event": "unsupported",
                "ts": round(time.time(), 3),
                "game": game_id,
                "turn": int(turn or 0),
                "why": f"{type(err).__name__}: {err}"[:160],
            }
    raise ValueError(f"unknown collector event: {kind!r}")


def _terminate_proc(proc: subprocess.Popen[str] | None) -> None:
    if proc is None or proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=2.0)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=2.0)


def serve_live(
    scorer: PolicyScorer,
    sock_path: Path,
    logs_dir: Path,
    bgbench_bin: Path = DEFAULT_BGBENCH,
    ref_cards: Path = DEFAULT_REF_CARDS,
    hs_cards: Path = DEFAULT_HS_CARDS,
    seed: int = 1,
) -> None:
    """Follow newest Power.log continuously and broadcast moves to socket clients."""
    server = SocketServer(sock_path, scorer.ckpt_name)
    stop_flag = False

    def handle_sig(_signum: int, _frame: Any) -> None:
        nonlocal stop_flag
        stop_flag = True

    old_int = signal.signal(signal.SIGINT, handle_sig)
    old_term = signal.signal(signal.SIGTERM, handle_sig)

    work_dir = Path(tempfile.mkdtemp(prefix="bg_live_actions_"))
    current_log: Path | None = None
    proc: subprocess.Popen[str] | None = None
    sel = selectors.DefaultSelector()
    last_scan = 0.0

    try:
        while not stop_flag:
            now = time.monotonic()
            if proc is None or (now - last_scan >= 1.0):
                last_scan = now
                newest = find_newest_log(logs_dir)
                if newest is not None and newest != current_log:
                    if proc is not None and proc.stdout is not None:
                        try:
                            sel.unregister(proc.stdout)
                        except KeyError:
                            pass
                    _terminate_proc(proc)
                    proc = None
                    scorer.clear_history()
                    current_log = newest
                    cmd = [
                        str(bgbench_bin),
                        "live-actions",
                        "--live-log",
                        str(current_log),
                        "--out",
                        str(work_dir),
                        "--cards",
                        str(hs_cards),
                    ]
                    proc = subprocess.Popen(
                        cmd,
                        cwd=str(ROOT),
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                        text=True,
                        bufsize=1,
                    )
                    assert proc.stdout is not None
                    sel.register(proc.stdout, selectors.EVENT_READ)

            if proc is None:
                time.sleep(0.25)
                continue

            events = sel.select(timeout=0.25)
            for key, _ in events:
                line = key.fileobj.readline()
                if not line:
                    try:
                        sel.unregister(key.fileobj)
                    except KeyError:
                        pass
                    _terminate_proc(proc)
                    if proc.returncode not in (0, None) and proc.stderr is not None:
                        err_text = proc.stderr.read().strip()
                        if err_text:
                            print(f"bgbench live-actions exited ({proc.returncode}): {err_text}", file=sys.stderr, flush=True)
                    proc = None
                    current_log = None
                    break
                text = line.strip()
                if not text:
                    continue
                ev = json.loads(text)
                msg = handle_collector_event(ev, scorer, ref_cards=ref_cards, seed=seed)
                server.broadcast(msg)
    finally:
        sel.close()
        _terminate_proc(proc)
        shutil.rmtree(work_dir, ignore_errors=True)
        server.close()
        signal.signal(signal.SIGINT, old_int)
        signal.signal(signal.SIGTERM, old_term)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--ckpt", required=True, help="checkpoint .pt file")
    ap.add_argument("--socket", default=str(default_socket()), help="Unix socket path")
    ap.add_argument("--logs", default=str(DEFAULT_LOGS), help="Hearthstone Logs folder")
    ap.add_argument("--top", type=int, default=3, help="number of top moves to report")
    ap.add_argument("--bgbench", default=str(DEFAULT_BGBENCH), help="path to bgbench binary")
    ap.add_argument("--seed", type=int, default=1, help="engine seed for live seating")
    ap.add_argument(
        "--once",
        default=None,
        help="score one Power.log snapshot, print JSON decision to stdout, and exit",
    )
    return ap.parse_args(argv)


def main(argv: list[str] | None = None) -> None:
    args = parse_args(argv)
    bgbench_bin = resolve_path(args.bgbench)
    scorer = PolicyScorer(Path(args.ckpt), top_k=args.top)
    if args.once:
        msg = run_once(
            resolve_path(args.once),
            scorer,
            bgbench_bin=bgbench_bin,
            seed=args.seed,
        )
        if msg is not None:
            print(json.dumps(msg, separators=(",", ":")))
        return
    serve_live(
        scorer,
        sock_path=Path(args.socket).expanduser(),
        logs_dir=Path(args.logs).expanduser(),
        bgbench_bin=bgbench_bin,
        seed=args.seed,
    )


if __name__ == "__main__":
    main()
