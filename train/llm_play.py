"""DeepSeek plays one seat of one game against seven of Era's real boards.

The learner seat is the only Python seat: `bgpy.VecEnv(1, seed, [1,0,0,0,0,7],
real_boards=...)`. Every time it is that seat's turn, the observation and the
legal-action mask go through `train/llm/describe.py` into plain text, and a
player answers with one action number. The whole exchange is logged, reasoning
included, in `train/runs/<name>/`.

Players:
  --player deepseek  the real thing, through River's OpenAI-compatible API
                     (reads the key from the environment, never logs it)
  --player fake      offline: a random legal action that prefers not to end the
                     turn, with made-up text, so the loop and the logs can be
                     tested without the network

After a game the player writes 3 to 5 lessons into `train/runs/llm-lessons.md`,
which goes into the system message of every later game.

    .venv/bin/python train/llm_play.py --player fake --games 2 --name llm-fake-test
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime
from pathlib import Path

import numpy as np

import bgpy

HERE = Path(__file__).resolve().parent
LLM_DIR = HERE / "llm"
sys.path.insert(0, str(LLM_DIR))
sys.path.insert(0, str(HERE))

import describe  # noqa: E402  (needs the path above)
import real_boards  # noqa: E402

RUNS = HERE / "runs"
LESSONS = RUNS / "llm-lessons.md"
PRIMER = LLM_DIR / "rules_primer.md"

LINEUP = [1, 0, 0, 0, 0, 7]  # one learner seat, seven real boards
API_URL = "https://api.river.ai/v1/chat/completions"
MODEL = "deepseek-ai/DeepSeek-V4.1-Flash-1017"
MAX_TOKENS = 8000
TIMEOUT = 300
RETRY_WAITS = (10, 30, 60, 120)
KEY_VAR = "RIVER_API_KEY"
LESSONS_CAP = 40

ANSWER_FORMAT = '''## How to answer

Answer with a JSON object and nothing else:

{"action": <one of the legal action numbers>, "plan": "<one or two sentences: your plan for this turn and for the game>"}

The number must be one of the actions listed under "Legal actions" in the message. "plan" is your own memory: it comes back to you at your next decision. Do not write anything outside the JSON.'''


class StopGame(Exception):
    """Stop cleanly: the API is out of credit, or it would not answer."""


def now():
    return datetime.now().strftime("%Y-%m-%d %H:%M:%S")


# --------------------------------------------------------------------------
# Players


class FakePlayer:
    """Offline stand-in: a random legal action that prefers not to end the
    turn, with made-up plan and reasoning, so the loop and logs can be
    tested without the network."""

    kind = "fake"

    PLANS = [
        "Buy a minion now and level next turn.",
        "Hold the Gold, keep the board, and level up soon.",
        "Take what the Tavern gives and keep the hand small.",
        "Play the cheap cards first and end the turn with a full board.",
    ]

    def __init__(self, rng, log):
        self.rng = rng
        self.log = log

    def answer(self, messages, legal, turn_actions):
        """(content, reasoning, usage) as the API would return them."""
        ends = [i for i in legal if describe.action_at(i)[0] == "end"]
        others = [i for i in legal if i not in ends]
        if others and len(turn_actions) < 6:
            action = int(self.rng.choice(others))
        elif ends:
            action = ends[0]
        else:
            action = int(self.rng.choice(legal))
        plan = self.rng.choice(self.PLANS)
        content = json.dumps({"action": action, "plan": plan})
        reasoning = (
            "Fake reasoning (offline player).\n"
            f"There are {len(legal)} legal actions. I took action {action} "
            f"because it is legal and this game is only a test of the loop.\n"
            f"My plan: {plan}"
        )
        usage = {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
        self.log.api("decision", messages, content, reasoning, usage, 0.0, action)
        return content, reasoning, usage

    def lessons(self, summary):
        self.log.api("lessons", [{"role": "user", "content": summary}],
                     "(fake lessons written below)", "", {}, 0.0)
        return (
            "- Fake lesson: buy minions before the Tavern is refreshed.\n"
            "- Fake lesson: level up when the upgrade is cheap.\n"
            "- Fake lesson: keep the board order in mind before the fight."
        )


class DeepSeekPlayer:
    """River's OpenAI-compatible endpoint, standard library only."""

    kind = "deepseek"

    def __init__(self, key, log):
        self.key = key
        self.log = log

    def _post(self, payload):
        request = urllib.request.Request(
            API_URL,
            data=json.dumps(payload).encode("utf-8"),
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self.key}",
            },
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=TIMEOUT) as response:
            return json.loads(response.read().decode("utf-8"))

    def call(self, messages, kind):
        """One API call with the brief's retries. Never logs the key."""
        payload = {
            "model": MODEL,
            "messages": messages,
            "reasoning_effort": "high",
            "max_tokens": MAX_TOKENS,
        }
        started = time.time()
        for attempt, wait in enumerate((0,) + RETRY_WAITS):
            if wait:
                self.log.note(f"{kind}: waiting {wait}s before retry {attempt}")
                time.sleep(wait)
            try:
                data = self._post(payload)
            except urllib.error.HTTPError as e:
                if e.code == 402:
                    raise StopGame(
                        "HTTP 402 from River: the credit is used up, stopping"
                    ) from None
                if e.code == 429 or e.code >= 500:
                    self.log.note(f"{kind}: HTTP {e.code}, will retry")
                    continue
                raise StopGame(f"HTTP {e.code} from River, stopping") from None
            except (urllib.error.URLError, TimeoutError, OSError) as e:
                self.log.note(f"{kind}: network error ({e}), will retry")
                continue
            message = (data.get("choices") or [{}])[0].get("message") or {}
            content = message.get("content") or ""
            reasoning = message.get("reasoning_content") or ""
            usage = data.get("usage") or {}
            self.log.api(kind, messages, content, reasoning, usage,
                         time.time() - started)
            return content, reasoning, usage
        raise StopGame(
            f"no answer from River after {len(RETRY_WAITS)} retries, stopping"
        )

    def answer(self, messages, legal, turn_actions):
        content, reasoning, usage = self.call(messages, "decision")
        return content, reasoning, usage

    def lessons(self, summary):
        messages = [
            {
                "role": "system",
                "content": (
                    "You have just finished a game of Hearthstone Battlegrounds. "
                    "Write 3 to 5 short lessons for your future games, in your own "
                    "words, one per line, each starting with \"- \". Only lessons, "
                    "no other text."
                ),
            },
            {"role": "user", "content": summary},
        ]
        content, reasoning, usage = self.call(messages, "lessons")
        self.log.api_note(f"lessons: {usage}")
        return content


# --------------------------------------------------------------------------
# Logs


class GameLog:
    """game-<n>.md for a human, game-<n>.jsonl for every API call."""

    def __init__(self, folder, game):
        self.md_path = folder / f"game-{game}.md"
        self.json_path = folder / f"game-{game}.jsonl"
        self.md = self.md_path.open("w")
        self.json = self.json_path.open("w")

    def write(self, text):
        self.md.write(text)
        self.md.flush()

    def note(self, text):
        """A line for both files (retries, stops, the lessons step)."""
        self.write(f"\n> log: {text}\n")
        self.api_note(text)

    def api(self, kind, messages, content, reasoning, usage, seconds,
            action=None):
        line = {
            "time": now(),
            "kind": kind,
            "seconds": round(seconds, 3),
            "messages": messages,
            "response": {
                "content": content,
                "reasoning_content": reasoning,
                "usage": usage,
            },
            "action": action,
        }
        self.json.write(json.dumps(line) + "\n")
        self.json.flush()

    def api_note(self, text):
        self.json.write(json.dumps({"time": now(), "kind": "note", "text": text}) + "\n")
        self.json.flush()

    def close(self):
        self.md.close()
        self.json.close()


# --------------------------------------------------------------------------
# Prompt pieces


def parse_answer(content, legal):
    """(action or None, plan, why-not) for one answer."""
    text = (content or "").strip()
    plan = ""
    if text.startswith("```"):
        text = "\n".join(
            line for line in text.splitlines() if not line.strip().startswith("```")
        ).strip()
    try:
        obj = json.loads(text)
    except json.JSONDecodeError:
        match = re.search(r"\{.*\}", text, re.S)
        if not match:
            return None, plan, "the answer was not JSON"
        try:
            obj = json.loads(match.group(0))
        except json.JSONDecodeError:
            return None, plan, "the answer was not JSON"
    if not isinstance(obj, dict):
        return None, plan, "the answer was not a JSON object"
    plan = str(obj.get("plan", ""))
    action = obj.get("action")
    if isinstance(action, str) and action.strip().lstrip("-").isdigit():
        action = int(action.strip())
    if isinstance(action, bool) or not isinstance(action, int):
        return None, plan, 'the answer had no whole number in "action"'
    if action not in legal:
        return None, plan, f"action {action} is not one of the legal numbers"
    return action, plan, ""


def read_lessons(cap=LESSONS_CAP):
    """The newest lessons from the notebook, as a bullet list."""
    if not LESSONS.exists():
        return ""
    bullets = [
        line.strip() for line in LESSONS.read_text().splitlines()
        if line.strip().startswith("- ")
    ]
    return "\n".join(bullets[-cap:])


def append_lessons(name, game, place, text):
    RUNS.mkdir(parents=True, exist_ok=True)
    lines = []
    for raw in (text or "").splitlines():
        line = raw.strip()
        if not line:
            continue
        if line.startswith("* "):
            line = "- " + line[2:]
        elif not line.startswith("- "):
            line = "- " + re.sub(r"^[0-9]+[.)]\s*", "", line).lstrip("-* ").strip()
        lines.append(line)
    with LESSONS.open("a") as f:
        f.write(f"\n## {now()}, {name} game {game}, place {place}\n")
        for line in lines:
            f.write(line + "\n")
    return lines


def system_message(primer, lessons):
    parts = [primer.strip()]
    if lessons:
        parts.append(
            "## Lessons from your earlier games\n\n"
            "You wrote these yourself, after earlier games. Use them if they help, "
            "but the rules above win where they disagree.\n\n" + lessons
        )
    parts.append(ANSWER_FORMAT)
    return "\n\n".join(parts)


def last_fight_line(desc, obs, prev_vitals):
    """One line on the fight that just happened: whom, the result, the cost.

    `prev_vitals` is the Health plus armor at the start of the last turn, so the
    difference is what the fight (and any card that costs Health) took. It is
    None on the first turn, when there has been no fight yet."""
    if prev_vitals is None:
        return ""
    won = float(obs[bgpy.SCALAR_WON_LAST_COMBAT]) != 0.0
    lost = float(obs[bgpy.SCALAR_LOST_LAST_COMBAT]) != 0.0
    result = "won" if won else ("lost" if lost else "tied")
    foe = ""
    for k in range(1, bgpy.PLAYERS):
        base = bgpy.OWN_END + (k - 1) * bgpy.OPP_BLOCK
        if float(obs[base + bgpy.OPP_LAST]) == 0.0:
            continue
        hero = (desc.hero_name(obs[base + bgpy.OPP_HERO])
                if obs[base + bgpy.OPP_HERO] else "")
        # The state text calls them "Opponent k", so the line does too.
        foe = f"opponent {k}" + (f" ({hero})" if hero else "")
        if float(obs[bgpy.SCALAR_GHOST]) != 0.0:
            foe += " playing as the ghost"
        break
    line = f"you fought {foe} and {result}" if foe else f"you {result} your fight"
    now = (int(round(float(obs[bgpy.SCALAR_HEALTH])))
           + int(round(float(obs[bgpy.SCALAR_ARMOR]))))
    delta = now - prev_vitals
    if delta < 0:
        line += f", and you took {-delta} damage"
    elif delta > 0:
        line += f", and you gained {delta} Health or armor"
    return line + "."


def user_message(state, plan, turn_actions, fight_line, actions):
    parts = ["## Current state\n", state, ""]
    parts.append("## Your plan so far\n")
    parts.append(plan.strip() if plan.strip() else "(you have not written a plan yet)")
    parts.append("")
    if fight_line:
        parts.append("## Your last fight\n")
        parts.append(fight_line)
        parts.append("")
    parts.append("## Actions you have taken this turn\n")
    if turn_actions:
        for n, (_index, words) in enumerate(turn_actions, 1):
            parts.append(f"{n}. {words}")
    else:
        parts.append("(none yet)")
    parts.append("")
    parts.append("## Legal actions\n")
    for index, words in actions:
        parts.append(f"{index}: {words}")
    parts.append("")
    parts.append("Which action? Answer with JSON only.")
    return "\n".join(parts)


# --------------------------------------------------------------------------
# One game


def play_game(game, args, boards, primer, lessons, player_factory):
    seed = args.seed + game - 1
    env = bgpy.VecEnv(1, seed, LINEUP, real_boards=boards)
    desc = describe.Descriptor(env)
    n = 1
    obs = np.zeros((n, bgpy.OBS_SIZE), np.float32)
    mask = np.zeros((n, bgpy.N_ACTIONS), bool)
    seat = np.zeros(n, np.int64)
    kind = np.zeros(n, np.int64)
    places = np.zeros((n, bgpy.PLAYERS), np.int8)
    tier_turns = np.zeros((n, bgpy.PLAYERS, bgpy.MAX_TIER), np.int8)
    tribes = np.zeros((n, bgpy.PLAYERS, bgpy.TRIBES), np.int8)
    reset = np.zeros(n, bool)

    args.out.mkdir(parents=True, exist_ok=True)
    log = GameLog(args.out, game)
    player = player_factory(log)
    system = system_message(primer, lessons)

    started = time.time()
    tokens = {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
    place = 0
    stopped = ""
    decisions = 0
    plan = ""
    turn = -1
    turn_actions = []
    fight_line = ""
    prev_vitals = None
    last_board = []
    final_obs = None
    history = []
    reason = ""

    log.write(f"# Game {game}, {now()}\n\n")
    log.write(f"- seed: {seed}\n- player: {player.kind}\n- name: {args.name}\n")
    log.write(f"- lineup: learner, then 7 real boards\n\n")

    try:
        while True:
            env.observe(obs, mask, seat, kind)
            who = int(seat[0])
            o, m = obs[0], mask[0]
            if int(kind[0]) != bgpy.LEARNER:
                log.note(f"the waiting seat {who} is not the learner; stopping")
                break
            this_turn = int(round(float(o[bgpy.SCALAR_TURN])))
            legal = [int(i) for i in np.nonzero(m)[0]]
            if not legal:
                reason = "no legal action at all; stopping"
                log.note(reason)
                break
            state, actions = desc.describe(o, m, who)
            final_obs = o.copy()

            new_turn = this_turn != turn
            if new_turn:
                turn = this_turn
                turn_actions = []
                health = int(round(float(o[bgpy.SCALAR_HEALTH])))
                armor = int(round(float(o[bgpy.SCALAR_ARMOR])))
                fight_line = last_fight_line(desc, o, prev_vitals)
                prev_vitals = health + armor
                log.write(
                    f"\n## Turn {this_turn} (decision {decisions + 1}): "
                    f"health {health}, armor {armor}, "
                    f"gold {int(round(float(o[bgpy.SCALAR_GOLD])))}, "
                    f"tier {int(round(float(o[bgpy.SCALAR_TIER])))}\n"
                )
            else:
                # The fight is news only on the first decision of a turn.
                fight_line = ""

            if decisions >= args.max_decisions:
                reason = f"stopped at the {args.max_decisions} decision cap without a final place"
                log.note(reason)
                break

            messages = [
                {"role": "system", "content": system},
                {"role": "user",
                 "content": user_message(state, plan, turn_actions, fight_line, actions)},
            ]
            # The state is in the md in full at the start of a turn and while a
            # choice is open; otherwise only what changed (the jsonl always has
            # all of it).
            show_state = new_turn or desc.has_choice(o, m)
            log.write(f"\n### Decision {decisions + 1} (turn {this_turn})\n\n")
            if show_state:
                log.write("State as the player saw it:\n\n```\n" + state + "\n"
                          + "\n".join(f"{i}: {w}" for i, w in actions) + "\n```\n")
            else:
                log.write(
                    "State (in full in the jsonl; what changed): "
                    f"gold {int(round(float(o[bgpy.SCALAR_GOLD])))}, "
                    f"hand {len(desc.hand_lines(o)[0])}, "
                    f"board {sum(1 for k in range(bgpy.BOARD) if desc.present(desc.slot(o, bgpy.BOARD_START, k)))}, "
                    f"{len(actions)} legal actions.\n"
                )
            if fight_line:
                log.write(f"\nLast fight: {fight_line}\n")

            content, reasoning, usage = player.answer(messages, legal, turn_actions)
            for key in tokens:
                tokens[key] += int(usage.get(key) or 0)
            action, answer_plan, why = parse_answer(content, legal)
            if action is None:
                # Ask once more, saying what was wrong.
                log.write(f"\nThe answer was not usable ({why}). Asking once more.\n")
                retry_messages = list(messages) + [
                    {"role": "assistant", "content": content},
                    {"role": "user",
                     "content": (f"That answer was not usable: {why}. Reply with JSON "
                                 "only, and with the number of one of the legal "
                                 f"actions: {{\"action\": <number>, \"plan\": \"...\"}}")},
                ]
                content2, reasoning2, usage2 = player.answer(retry_messages, legal,
                                                             turn_actions)
                for key in tokens:
                    tokens[key] += int(usage2.get(key) or 0)
                reasoning = (reasoning + "\n\n[after the retry]\n" + reasoning2).strip()
                content = content2
                action, answer_plan, why = parse_answer(content, legal)
            fallback = ""
            if action is None:
                ends = [i for i in legal if describe.action_at(i)[0] == "end"]
                action = ends[0] if ends else (legal[0] if legal else None)
                fallback = why
                log.write(f"\nFallback: {why}. Taking action {action}.\n")
            if action is None:
                reason = "no legal action at all; stopping"
                log.note(reason)
                break

            plan = answer_plan or plan
            words = dict(actions).get(action, desc.action_words(o, action))
            turn_actions.append((action, words))
            history.append({"turn": this_turn, "action": action, "words": words,
                            "why": fallback})
            print(f"  turn {this_turn}, decision {decisions + 1}: {action}: {words}",
                  flush=True)

            log.write("\nReasoning:\n\n")
            for line in (reasoning or "(none given)").splitlines():
                log.write(f"> {line}\n")
            log.write(f"\nAnswer: {action}: {words}\n")
            if plan:
                log.write(f"\nPlan: {plan}\n")
            if fallback:
                log.write(f"\n(fallback: {fallback})\n")
            log.write("\n")
            log.json.write(json.dumps({
                "time": now(), "kind": "choice", "turn": this_turn,
                "action": action, "words": words, "plan": plan,
                "fallback": fallback,
            }) + "\n")
            log.json.flush()

            decisions += 1
            last_board = desc.board_lines(o, bgpy.BOARD_START, bgpy.BOARD)

            before = places.copy()
            env.step(np.array([action]), places, tier_turns, tribes, reset)
            if places[0, who] and not before[0, who]:
                place = int(places[0, who])
                log.write(f"\n## The game ended: place {place} of 8\n\n")
                log.write("Your final board, as it was when you last acted:\n\n")
                log.write("\n".join(last_board) if last_board else "  (empty)")
                log.write("\n\n")
                break
            if bool(reset[0]):
                reason = "the game ended without a place for the learner"
                log.note(reason)
                break
    except StopGame as e:
        stopped = str(e)
        log.note(f"stopped: {stopped}")

    seconds = time.time() - started

    # The lessons step.
    lessons_written = []
    if place and not args.no_lessons and not stopped:
        summary = lessons_request(desc, final_obs, place, last_board)
        try:
            text = player.lessons(summary)
            lessons_written = append_lessons(args.name, game, place, text)
            log.write("\n## Lessons it wrote\n\n")
            for line in lessons_written:
                log.write(f"{line}\n")
        except StopGame as e:
            stopped = stopped or str(e)
            log.note(f"no lessons: {stopped}")

    log.write("\n## End of game\n\n")
    log.write(f"- place: {place if place else 'none (unfinished)'}\n")
    log.write(f"- decisions: {decisions}\n")
    log.write(f"- wall time: {seconds:.1f} s\n")
    log.write(f"- tokens: {tokens['prompt_tokens']} in, "
              f"{tokens['completion_tokens']} out, {tokens['total_tokens']} total\n")
    if history:
        log.write("\nWhat it did, in order:\n\n")
        for h in history:
            log.write(f"- turn {h['turn']}: {h['words']}\n")
    if stopped:
        log.write(f"\nstopped: {stopped}\n")
    log.close()
    return {
        "game": game,
        "place": place,
        "decisions": decisions,
        "seconds": seconds,
        "tokens": dict(tokens),
        "lessons": lessons_written,
        "stopped": stopped,
        "reason": reason,
    }


def lessons_request(desc, obs, place, last_board):
    """The message the player reflects on: the result and the boards that beat it."""
    if place == 1:
        lines = ["I won this game of Hearthstone Battlegrounds: place 1 of 8."]
    else:
        lines = [f"I finished place {place} of 8 in this game of Hearthstone Battlegrounds."]
    lines.append("")
    lines.append("My board when I last acted:")
    lines.extend(last_board if last_board else ["  (empty)"])
    lines.append("")
    lines.append("The players that beat me, as I last saw them:")
    listed = 0
    for k in range(1, bgpy.PLAYERS):
        base = bgpy.OWN_END + (k - 1) * bgpy.OPP_BLOCK
        alive = float(obs[base + bgpy.OPP_ALIVE]) != 0.0
        other = int(round(float(obs[base + bgpy.OPP_PLACEMENT])))
        # Still alive means still ahead of me; a placement is only known once
        # the player is out.
        if place == 1 or not (alive or (other and other < place)):
            continue
        listed += 1
        hero = (desc.hero_name(obs[base + bgpy.OPP_HERO])
                if obs[base + bgpy.OPP_HERO] else "an unrecorded hero")
        tier = int(round(float(obs[base + bgpy.OPP_TIER])))
        head = f"- {hero}, tier {tier}"
        if not alive:
            head += f", finished {other}"
        lines.append(head)
        board = desc.board_lines(obs, base + bgpy.OPP_SCALARS, bgpy.BOARD, indent="    ",
                                 own=False)
        lines.extend(board if board else ["    (no board seen)"])
    if not listed:
        lines.append("(nobody: I won)" if place == 1 else "(no board recorded)")
    lines.append("")
    lines.append("Write 3 to 5 short lessons for my future games, one per line, "
                 "each starting with \"- \".")
    return "\n".join(lines)


# --------------------------------------------------------------------------
# Main


def parse_args(argv=None):
    a = argparse.ArgumentParser(description="One LLM seat against seven real boards.")
    stamp = datetime.now().strftime("%Y-%m-%d-%H%M")
    a.add_argument("--name", default=f"llm-{stamp}",
                   help="the folder under train/runs/ for the logs")
    a.add_argument("--games", type=int, default=1, help="number of games, one after another")
    a.add_argument("--seed", type=int, default=1)
    a.add_argument("--player", choices=("deepseek", "fake"), default="deepseek")
    a.add_argument("--no-lessons", action="store_true",
                   help="skip the lessons step after each game")
    a.add_argument("--max-decisions", type=int, default=400,
                   help="safety cap on decisions per game")
    args = a.parse_args(argv)
    args.out = RUNS / args.name
    return args


def main(argv=None):
    args = parse_args(argv)
    key = ""
    if args.player == "deepseek":
        key = os.environ.get(KEY_VAR, "")
        if not key:
            raise SystemExit(
                f"{KEY_VAR} is not set. DeepSeek needs it; it is read from the "
                "environment only and never written anywhere. Use --player fake "
                "to test the harness offline."
            )
    primer = PRIMER.read_text()
    lessons = "" if args.no_lessons else read_lessons()

    boards, unknown = real_boards.load(real_boards.DEFAULT_DIR)
    if not boards:
        raise SystemExit(f"no real boards in {real_boards.DEFAULT_DIR}")
    print(f"real boards from {real_boards.DEFAULT_DIR}:")
    real_boards.print_summary(boards, unknown)

    def player_factory(log):
        if args.player == "fake":
            return FakePlayer(np.random.default_rng(args.seed), log)
        return DeepSeekPlayer(key, log)

    print(f"logging to {args.out}", flush=True)
    results = []
    for game in range(1, args.games + 1):
        print(f"\n=== game {game} of {args.games} ===", flush=True)
        result = play_game(game, args, boards, primer, lessons, player_factory)
        results.append(result)
        if result["place"]:
            print(f"game {game}: place {result['place']}, {result['decisions']} "
                  f"decisions, {result['seconds']:.0f} s, "
                  f"{result['tokens']['total_tokens']} tokens", flush=True)
        else:
            print(f"game {game}: no place ({result['reason'] or result['stopped']})",
                  flush=True)
        if result["stopped"]:
            print(f"stopping the run: {result['stopped']}", flush=True)
            break
        # A new game sees the lessons the last one wrote.
        lessons = "" if args.no_lessons else read_lessons()

    placed = [r["place"] for r in results if r["place"]]
    if len(placed) > 1:
        print(f"\naverage place: {sum(placed) / len(placed):.2f} over {len(placed)} games")
    return 0


if __name__ == "__main__":
    sys.exit(main())
