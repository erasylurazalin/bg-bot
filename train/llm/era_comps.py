#!/usr/bin/env python3
"""DeepSeek reads Era's own games and finds the comps.

Two passes, every call logged:

1. One call per game: the game's digest (train/llm/digest.py) goes to the
   model with a system message that says what Battlegrounds is and that the
   game is one real player's (Era's). It answers a JSON summary of the game:
   the comp, the core, when Era committed, the pivots, the level-up turns,
   what went well and badly, and a few lessons learned from Era that game.
   If it asks for more detail on one turn (`{"need_turn": N}`) it gets that
   turn's raw actions and answers once more.
2. One call for everything, or one per chunk of games when the summaries do
   not fit, then a merge call: the comp list in the shape of comps.toml with
   Era's games, places and patch split per comp, Era's habits, the per-game
   lessons merged into one list, and the things worth asking Era.

Files, in train/runs/<name>/:

- game-<source>-g<n>.json: the pass-1 answer and its metadata.
- game-<source>-g<n>.md: the full prompt, the full reasoning and the answer
  of every call for that game, and the game's lessons at the end.
- pass2*.md: the same for the pass-2 calls.
- calls.jsonl: every request and response, one line each.
- comps-era.toml, report.md, lessons-from-era.md: the pass-2 output.

The numbers per comp (games, average place, top-4 rate, patch split) and per
lesson (how many games, average place) are computed from the pass-1 files,
not taken from the model.

Offline test: `--player fake` answers with canned JSON and fake reasoning and
never touches the network. The key comes from RIVER_API_KEY only and is never
printed or written.

    python3 -I train/llm/era_comps.py --player fake --games 5
    python3 -I train/llm/era_comps.py --resume --pass2-only
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import re
import sys
import threading
import time
import urllib.error
import urllib.request
from collections import Counter, defaultdict
from datetime import datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
sys.dont_write_bytecode = True  # leave no __pycache__ next to the tools

import digest  # noqa: E402  (same folder)

RUNS = ROOT / "train" / "runs"
PRIMER = HERE / "rules_primer.md"
# Era's own strategy notes (scam and the like), private, in docs/notes/
STRATEGY = HERE.parent.parent / "docs" / "notes" / "era-strategy.md"


def strategy_text() -> str:
    if not STRATEGY.exists():
        return ""
    return ("## How Era plays (Era's own notes)\n\n" + STRATEGY.read_text().strip() +
            "\n\nA game can end in a scam instead of a comp: when the final board is "
            "built from scam pieces (Venomous, Poisonous, Heroic Underdog, Leeroy) "
            "rather than a scaling engine, name the comp \"Scam\" (\"Scam (after a "
            "failed X)\" if Era first tried X), and say from which turn Era switched "
            "to scamming. You don't see the opponents' boards: judge the scam by "
            "what Era bought and kept, not by the fights.")
COMPS_TOML = ROOT / "comps.toml"

API_URL = "https://api.river.ai/v1/chat/completions"
MODEL = "deepseek-ai/DeepSeek-V4.1-Flash-1017"
MAX_TOKENS = 16000  # as in train/llm_play.py
# pass 2 writes the whole comp list, habits and lessons in one answer: 16000
# cut the first real run off mid-JSON (2026-11-06)
PASS2_MAX_TOKENS = 64000
TIMEOUT = 900  # pass 2 answers run long
RETRY_WAITS = (10, 30, 60, 120)
KEY_VAR = "RIVER_API_KEY"

# How much of one pass-2 prompt to fill before it is split into chunks, in
# characters of game summaries (about 4 characters to a token).
PASS2_BUDGET = 120_000

PASS1_INSTRUCTIONS = '''## The job

The message after this one is a digest of ONE finished game of Hearthstone
Battlegrounds, played by ONE real player, Era. It is Era's side of the game:
Era's shop, actions, board and hand, and whether the fight after each turn was
won. The opponents' boards are not in it, only the fights' results.

Era uses they/them pronouns: write about them as "Era" or with they, them and
their, never he, she, his or her, in every field of the answer.

Look at the actions Era took, and describe the game:

- Name the comp the game ended up as, in a few words ("Undead Reborn",
  "Mech Magnetic", "Quilboar Blood Gems", "Menagerie spells").
- Name its core: the cards the final board was built around, not every card
  that passed through.
- Say when Era committed to it (the turn), and whether they pivoted.
- Note the patch: a game played before patch 37.0.2 says so at the top and
  lists the numbers that patch changed. That does not make the game wrong; it
  only means some numbers (stats, prices) differ from today's.
- The digest's "Lobby tribes" line is the lobby's five types. The "tribes"
  field of your answer is NOT the lobby: it is the comp's own types, the types
  of the minions the comp is built around (usually one or two; [] for a comp of
  minions with no type).
- Do not judge the player harshly: say what Era did and what it looks like it
  was for. Era is a strong player and these are their real games.

Also write "lessons": 3 to 5 lessons learned from Era in this game, in your
own words. Each one concrete: the turn, the cards, the situation. Both kinds:
decisions worth copying and mistakes worth avoiding. They are read by a bot
that trains on Era's games, so make them things that could be done again.

Answer with ONE JSON object and nothing else:

{"comp": "<short name>", "tribes": ["<tribe>", ...], "core": ["<card name>", ...],
 "golden_targets": ["<card Era wanted golden>", ...],
 "committed_turn": <turn number or null>,
 "pivots": [{"turn": <turn>, "from": "<comp or plan before>", "to": "<comp or plan after>", "why": "<what the actions suggest>"}],
 "level_turns": {"<tier>": <turn Era reached that tier>, ...},
 "key_decisions": ["<the decisions that shaped the game>", ...],
 "went_well": ["...", ...],
 "went_badly": ["...", ...],
 "place": <Era's final place, or 0 if the digest says it is unknown>,
 "patch": "37.0" or "37.0.2" or "unknown",
 "confidence": "high" or "medium" or "low",
 "notes": "<anything else worth knowing, or an empty string>",
 "lessons": ["<lesson 1>", "<lesson 2>", "<lesson 3>"]}

If the digest genuinely leaves a turn unclear and one more look at that turn's
raw actions would change your answer, answer exactly {"need_turn": <turn
number>} instead, and you will get that turn once. Use this at most once and
only when it matters.'''

PASS1_TURN_FOLLOWUP = '''That turn is below, as the collector wrote it: every step Era took, with what
it paid, and the state at the end of the turn. The start of the turn is the
state before its first step.

Answer the same JSON object as before, and nothing else.'''

PASS2_INSTRUCTIONS = '''## The job

You are given summaries of many of Era's own Battlegrounds games, each written
from a digest of that game by the same method. Merge them into a picture of
how Era plays. Era uses they/them pronouns: write about them as "Era" or with
they, them and their, never he, she, his or her. Do not judge the player
harshly. Only Era's games and Era's
decisions are in the data; opponents' boards are not analysed.

Produce:

a) A comp list in the shape of comps.toml (its header is above: tribes, core,
   golden, addons, when Era commits). One entry per comp Era actually played,
   with Era's own names for the cards, and the games that belong to it. Give
   each comp's games as their ids (they are in the summaries). Fill in what
   the data supports; leave a field out or empty where it does not.
b) Era's habits: their level-up turns (typical and spread), how often they
   refresh per turn by tier, how often and when they pivot, and what the top-4
   games do differently from the bottom-4 games. The data block at the
   top of the message already holds the counts and averages; read them, do
   not recount.
c) Lessons learned from Era: merge the per-game lessons into one list.
   Combine the ones that say the same thing (repeats combined into one line in
   your own words, with the card names and turns that matter), and give each
   merged lesson the ids of every game it came from. Keep lessons drawn from
   top-4 games and lessons drawn from bottom-4 games apart where they differ,
   and say which is which in the text of the lesson if it is not obvious.
d) A short list of things worth asking Era: comps you were not sure about,
   and why.

Answer with ONE JSON object and nothing else:

{"comps": [{"name": "<short name>", "tribes": ["<tribe>", ...],
            "summary": "<one line>", "core": ["<card>", ...],
            "golden": ["<card>", ...], "addons": ["<card>", ...],
            "commit": "<when Era commits, in words>",
            "plan": "<how the comp is played, in a few lines>",
            "confidence": "high" or "medium" or "low",
            "games": ["<game id>", ...]}],
 "habits": {"level_up_turns": "<typical turns and the spread>",
            "refreshes": "<how often Era refreshes, by tier>",
            "pivots": "<how often and when Era pivots>",
            "top4_vs_bottom4": "<what the top-4 games do differently>"},
 "lessons": [{"lesson": "<the merged lesson, in your own words>",
              "games": ["<game id>", ...],
              "kind": "copy" or "avoid"}],
 "ask_era": ["<question>", ...]}

The game ids are the "<source>-g<N>" strings at the start of each summary.
Every comp's and every lesson's games must be ids from the summaries.'''


class StopGame(Exception):
    """Stop cleanly: the API is out of credit, or it would not answer."""


def now() -> str:
    return datetime.now().strftime("%Y-%m-%d %H:%M:%S")


# --------------------------------------------------------------------------
# Logging


class RunLog:
    """calls.jsonl plus one .md per call, both flushed as the run goes."""

    def __init__(self, folder: Path):
        self.folder = folder
        self.folder.mkdir(parents=True, exist_ok=True)
        self.path = folder / "calls.jsonl"
        self.lock = threading.Lock()
        self.calls = 0
        self.prompt_tokens = 0
        self.completion_tokens = 0

    def record(self, kind: str, messages, content, reasoning, usage, seconds,
               game: str | None = None, note: str | None = None) -> None:
        usage = usage or {}
        line = {
            "time": now(),
            "kind": kind,
            "game": game,
            "seconds": round(seconds, 3),
            "messages": messages,
            "response": {
                "content": content,
                "reasoning_content": reasoning,
                "usage": usage,
            },
        }
        if note:
            line["note"] = note
        with self.lock:
            with self.path.open("a") as f:
                f.write(json.dumps(line) + "\n")
            self.calls += 1
            self.prompt_tokens += int(usage.get("prompt_tokens") or 0)
            self.completion_tokens += int(usage.get("completion_tokens") or 0)

    def note(self, text: str) -> None:
        with self.lock:
            with self.path.open("a") as f:
                f.write(json.dumps({"time": now(), "kind": "note", "text": text}) + "\n")

    def markdown(self, path: Path, title: str, calls, lessons=None) -> None:
        """Write one .md with every call of that step: prompt, reasoning,
        answer, and (pass 1) the game's lessons at the end."""
        rows = [f"# {title}", ""]
        for call in calls:
            rows.append(f"## {call['kind']} call, {call['time']}, "
                        f"{call['seconds']} s")
            if call.get("game"):
                rows.append("")
                rows.append(f"Game: {call['game']}")
            rows.append("")
            rows.append("### Prompt")
            rows.append("")
            for message in call["messages"]:
                rows.append(f"**{message['role']}**")
                rows.append("")
                rows.append("```text")
                rows.append(message["content"])
                rows.append("```")
                rows.append("")
            rows.append("### Reasoning")
            rows.append("")
            rows.append("```text")
            rows.append((call["response"].get("reasoning_content") or "").strip())
            rows.append("```")
            rows.append("")
            rows.append("### Answer")
            rows.append("")
            rows.append("```json")
            rows.append((call["response"].get("content") or "").strip())
            rows.append("```")
            rows.append("")
            usage = call["response"].get("usage") or {}
            rows.append(f"Usage: {usage}")
            rows.append("")
        if lessons:
            rows.append("## Lessons from this game")
            rows.append("")
            for lesson in lessons:
                rows.append(f"- {lesson}")
            rows.append("")
        with self.lock:
            path.write_text("\n".join(rows))


# --------------------------------------------------------------------------
# Players (the API client is copied from train/llm_play.py, which cannot be
# imported without a built bgpy; this is the small part that is needed)


class DeepSeekPlayer:
    """River's OpenAI-compatible endpoint, standard library only."""

    kind = "deepseek"

    def __init__(self, key: str, log: RunLog):
        self.key = key
        self.log = log

    def _post(self, payload: dict) -> dict:
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

    def call(self, messages, kind: str, context: dict | None = None):
        """(content, reasoning, usage). Never logs the key."""
        payload = {
            "model": MODEL,
            "messages": messages,
            "reasoning_effort": "high",
            "max_tokens": PASS2_MAX_TOKENS if kind.startswith("pass2") else MAX_TOKENS,
        }
        for attempt, wait in enumerate((0,) + RETRY_WAITS):
            if wait:
                self.log.note(f"{kind}: waiting {wait}s before retry {attempt}")
                time.sleep(wait)
            try:
                data = self._post(payload)
            except urllib.error.HTTPError as error:
                if error.code == 402:
                    raise StopGame(
                        "HTTP 402 from River: the credit is used up, stopping"
                    ) from None
                if error.code == 429 or error.code >= 500:
                    self.log.note(f"{kind}: HTTP {error.code}, will retry")
                    continue
                raise StopGame(f"HTTP {error.code} from River, stopping") from None
            except (urllib.error.URLError, TimeoutError, OSError) as error:
                self.log.note(f"{kind}: network error ({error}), will retry")
                continue
            message = (data.get("choices") or [{}])[0].get("message") or {}
            content = message.get("content") or ""
            reasoning = message.get("reasoning_content") or ""
            usage = data.get("usage") or {}
            return content, reasoning, usage
        raise StopGame(
            f"no answer from River after {len(RETRY_WAITS)} retries, stopping"
        )


class FakePlayer:
    """Offline stand-in: canned pass-1 and pass-2 JSON plus fake reasoning.

    The answers are made from the message it is given (the digest's place,
    tribes and card names), so the whole loop, the files and the statistics
    can be tested without the network."""

    kind = "fake"

    LESSONS_TOP4 = [
        "Level to tier 3 on turn 5 while the board still holds: the tempo loss "
        "is paid back by the tier 3 shop.",
        "Keep one scaling card ({card}) and feed it every Blood Gem and spell "
        "instead of spreading them over the board.",
        "Buy the triple when the third copy is in the shop, even if the pair "
        "has to sit in hand a turn: {card} golden is worth more than 3 Gold.",
        "Sell the token before playing the Battlecry, so the Battlecry has a "
        "free place to land.",
        "Freeze a two-card shop when both cards fit the comp; the next turn's "
        "Gold is better spent levelling.",
    ]
    LESSONS_BOTTOM4 = [
        "Do not buy tier 1 pairs after turn 6: the third copy never came and "
        "the Gold was gone.",
        "Refresh less at tier 4: 6 Gold of refreshes on turn 8 for no card "
        "that stayed on the board.",
        "Do not move the Divine Shield minion to the front before the fight: "
        "it ate the first attack and the Reborn stayed in the back.",
        "Commit to one tribe by turn 7, or the board stays a mix of strong "
        "singles that does not scale.",
        "Buy the trinket that gives economy (the gold or refresh one) before "
        "the fight-only trinket when the board is already strong.",
    ]

    def __init__(self, log: RunLog):
        self.log = log

    def call(self, messages, kind: str, context: dict | None = None):
        if kind.startswith("pass2"):
            return self.pass2(messages, context or {})
        return self.pass1(messages, context or {})

    # -- reading the digest ---

    @staticmethod
    def _place(text: str) -> int:
        found = re.search(r"Final place: (\d+) of 8", text)
        return int(found.group(1)) if found else 0

    @staticmethod
    def _tribes(text: str) -> list[str]:
        found = re.search(r"Lobby tribes[^:]*: ([a-z, ]+)", text)
        if not found:
            return []
        return [t.strip() for t in found.group(1).split(",") if t.strip()]

    @staticmethod
    def _patch(text: str) -> str:
        found = re.search(r"Patch: (37\.0\.2|37\.0|unknown)", text)
        return found.group(1) if found else "unknown"

    @staticmethod
    def _cards(text: str) -> list[str]:
        names = []
        minions = text.split("### Minions")[-1] if "### Minions" in text else ""
        for line in minions.splitlines():
            found = re.match(r"- ([^(]+) \(", line)
            if found:
                names.append(found.group(1).strip())
            elif names:
                break
        return names

    # -- pass 1 ---

    def pass1(self, messages, context):
        text = messages[-1]["content"]
        place = self._place(text)
        tribes = self._tribes(text)
        cards = self._cards(text)
        turns = int(context.get("turns") or 0)
        followup = bool(context.get("followup"))
        key = str(context.get("game") or "")
        if not followup and turns >= 8 and sum(map(ord, key)) % 3 == 0:
            # Exercise the one follow-up now and then: ask for a middle turn.
            turn = max(2, turns // 2)
            content = json.dumps({"need_turn": turn})
            reasoning = (
                "Fake reasoning (offline player).\n"
                f"The digest of {context.get('game')} leaves turn {turn} "
                "unclear: the actions look like a pivot but the digest does "
                "not say what the board was before it. I will ask for that "
                "turn's raw actions once."
            )
            return content, reasoning, {}
        tribe = tribes[0] if tribes else "menagerie"
        comp = f"{tribe.capitalize()} comp"
        core = cards[:3] or ["a strong single"]
        golden = [cards[0]] if cards else []
        place_value = place if place else 0
        lessons = (self.LESSONS_TOP4 if place_value and place_value <= 4
                   else self.LESSONS_BOTTOM4)[:4]
        lessons = [lesson.format(card=core[0]) for lesson in lessons]
        answer = {
            "comp": comp,
            "tribes": [tribe],
            "core": core,
            "golden_targets": golden,
            "committed_turn": 6 if turns >= 6 else None,
            "pivots": [
                {"turn": 7, "from": "mixed board", "to": comp,
                 "why": "the last three buys are all of one tribe"}
            ] if turns >= 7 else [],
            "level_turns": {"2": 3, "3": 5, "4": 7, "5": 9},
            "key_decisions": [
                f"bought {core[0]} early",
                "levelled to tier 3 on turn 5",
            ],
            "went_well": ["kept the board wide while levelling"],
            "went_badly": ["refreshed twice at tier 4 for nothing"],
            "place": place_value,
            "patch": self._patch(text),
            "confidence": "low",
            "notes": "Fake answer written offline to test the loop.",
            "lessons": lessons,
        }
        reasoning = (
            "Fake reasoning (offline player).\n"
            f"This is a test of the loop, not an analysis: {context.get('game')} "
            f"looks like a {comp} with {len(cards)} cards named in the digest, "
            f"ending {place_value or 'unplaced'}.\n"
            "My plan: reply with the JSON the brief asks for, with the fields "
            "filled from the digest's own header and cards."
        )
        return json.dumps(answer), reasoning, {}

    # -- pass 2 ---

    def pass2(self, messages, context):
        ids = list(context.get("ids") or [])
        places = context.get("places") or {}
        merged = bool(context.get("merge"))
        top = [i for i in ids if (places.get(i) or 8) <= 4]
        bottom = [i for i in ids if (places.get(i) or 8) > 4]
        comps = []
        if top:
            comps.append({
                "name": "Undead Reborn",
                "tribes": ["undead"],
                "summary": "Reborn Undead that come back bigger",
                "core": ["Eternal Knight", "Mummifier", "Titus Rivendare"],
                "golden": ["Eternal Knight"],
                "addons": ["Cadaver Caretaker"],
                "commit": "turn 6, once two Reborn Undead are on the board",
                "plan": "Level to 4 on schedule, keep the Eternal Knights, "
                        "and let the Deathrattles carry the midgame.",
                "confidence": "medium",
                "games": top,
            })
        if bottom:
            comps.append({
                "name": "Menagerie",
                "tribes": [],
                "summary": "Strong single cards, no scaling",
                "core": ["Brann Bronzebeard", "Gatekeeper Amalgam"],
                "golden": [],
                "addons": ["Hot-Air Surveyor"],
                "commit": "never really committed",
                "plan": "Buy the best card in the Tavern each turn.",
                "confidence": "low",
                "games": bottom,
            })
        lessons = []
        if top:
            lessons.append({
                "lesson": self.LESSONS_TOP4[0].format(card="Eternal Knight"),
                "games": top,
                "kind": "copy",
            })
        if bottom:
            lessons.append({
                "lesson": self.LESSONS_BOTTOM4[3].format(card="Gatekeeper Amalgam"),
                "games": bottom,
                "kind": "avoid",
            })
        answer = {
            "comps": comps,
            "habits": {
                "level_up_turns": "tier 2 on turn 3, tier 3 on turn 5, "
                                  "tier 4 on turn 7, tier 5 on turn 9",
                "refreshes": "about one Refresh a turn at tier 2, two at tier "
                             "3, four at tier 4",
                "pivots": "one pivot around turn 7 in most games",
                "top4_vs_bottom4": "top-4 games level on time and keep a "
                                   "scaling card; bottom-4 games refresh more "
                                   "at tier 4",
            },
            "lessons": lessons,
            "ask_era": [
                "Which comp was the turn 5 triple for in the games with no "
                "clear tribe?",
            ],
        }
        reasoning = (
            "Fake reasoning (offline pass 2).\n"
            f"{'Merging the partial analyses' if merged else 'Reading the summaries'}: "
            f"{len(ids)} games, {len(top)} top-4 and {len(bottom)} bottom-4.\n"
            "The canned answer puts the top-4 games in one comp and the rest "
            "in a second one, so the script's own statistics have something "
            "to count."
        )
        return json.dumps(answer), reasoning, {}


# --------------------------------------------------------------------------
# JSON from a model


def parse_json(content: str) -> dict | None:
    """The JSON object in a model's answer, fences and prose tolerated."""
    text = (content or "").strip()
    if text.startswith("```"):
        text = re.sub(r"^```[a-zA-Z]*\s*", "", text)
        text = re.sub(r"```\s*$", "", text).strip()
    try:
        value = json.loads(text)
        return value if isinstance(value, dict) else None
    except json.JSONDecodeError:
        pass
    start = text.find("{")
    end = text.rfind("}")
    if start >= 0 and end > start:
        try:
            value = json.loads(text[start:end + 1])
            return value if isinstance(value, dict) else None
        except json.JSONDecodeError:
            return None
    return None


def ask(player, log: RunLog, messages, kind: str, game=None, context=None,
        repair_retry=True):
    """One call, recorded in calls.jsonl; returns (content, reasoning, usage,
    seconds). A second call is allowed when the answer is not JSON."""
    started = time.time()
    content, reasoning, usage = player.call(messages, kind, context)
    seconds = time.time() - started
    log.record(kind, messages, content, reasoning, usage, seconds, game=game)
    if not repair_retry or parse_json(content) is not None:
        return content, reasoning, usage, seconds
    messages = list(messages) + [
        {"role": "assistant", "content": content},
        {"role": "user", "content": (
            "That answer is not a JSON object. Answer again with one JSON "
            "object and nothing else, in the shape asked for."
        )},
    ]
    started = time.time()
    content, reasoning, usage = player.call(
        messages, kind + "-repair", context)
    seconds = time.time() - started
    log.record(kind + "-repair", messages, content, reasoning, usage, seconds,
               game=game, note="the answer was not JSON")
    return content, reasoning, usage, seconds


# --------------------------------------------------------------------------
# Pass 1


def pass1_messages(primer: str, digest_text: str) -> list[dict]:
    return [
        {"role": "system", "content": primer.strip() + "\n\n" + strategy_text() + "\n\n" + PASS1_INSTRUCTIONS},
        {"role": "user", "content": (
            "The digest of the game follows.\n\n" + digest_text
        )},
    ]


def pass1_game(info: dict, player, log: RunLog, folder: Path) -> dict:
    """One game: the digest, one call (and at most one follow-up), the files."""
    key = info["key"]
    json_path = folder / f"game-{key}.json"
    if not info["digest_path"].exists():
        info["digest_path"].parent.mkdir(parents=True, exist_ok=True)
        info["digest_path"].write_text(digest.digest(info["path"], info))
    digest_text = info["digest_path"].read_text()

    primer = PRIMER.read_text()
    messages = pass1_messages(primer, digest_text)
    game = digest.load_json(info["path"])
    context = {"game": key, "turns": info["turns"], "place": info["place"]}

    calls = []
    content, reasoning, usage, seconds = ask(player, log, messages, "pass1",
                                            game=key, context=context)
    calls.append({"kind": "pass1", "time": now(), "seconds": round(seconds, 2),
                  "messages": messages, "game": key,
                  "response": {"content": content, "reasoning_content": reasoning,
                               "usage": usage}})
    answer = parse_json(content)
    asked_turn = None
    if answer is not None and "need_turn" in answer and "comp" not in answer:
        try:
            asked_turn = int(answer["need_turn"])
        except (TypeError, ValueError):
            asked_turn = None
    if asked_turn is not None:
        raw = digest.raw_turn(game, asked_turn)
        followup = list(messages) + [
            {"role": "assistant", "content": content},
            {"role": "user", "content": PASS1_TURN_FOLLOWUP + "\n\n" + raw},
        ]
        content2, reasoning2, usage2, seconds2 = ask(
            player, log, followup, f"pass1-turn{asked_turn}", game=key,
            context={"game": key, "turns": info["turns"], "followup": True})
        calls.append({"kind": f"pass1-turn{asked_turn}", "time": now(),
                      "seconds": round(seconds2, 2), "messages": followup,
                      "game": key,
                      "response": {"content": content2,
                                   "reasoning_content": reasoning2,
                                   "usage": usage2}})
        answer = parse_json(content2)
        if answer is not None and "need_turn" in answer and "comp" not in answer:
            answer = None

    result = {
        "key": key,
        "source": info["source"],
        "game": info["game"],
        "hero": info["hero"],
        "place": info["place"],
        "unfinished": info["unfinished"],
        "patch": info["patch"],
        "build": info["build"],
        "build_from": "power log" if info["from_log"] else "date guess",
        "lobby": info.get("lobby") or [],
        "date": info["date"],
        "turns": info["turns"],
        "steps": info["steps"],
        "digest_tokens": digest.size_estimate(digest_text),
        "followup_turn": asked_turn,
        "model": type(player).kind,
        "seconds": round(sum(c["seconds"] for c in calls), 2),
        "answer": answer,
        "raw_answer": None if answer is not None else content,
        "lessons": (answer or {}).get("lessons") or [],
    }
    log.markdown(folder / f"game-{key}.md",
                 f"{key}: {info['hero']}, place {info['place']}, patch {info['patch']}",
                 calls, lessons=result["lessons"])
    json_path.write_text(json.dumps(result, indent=1))
    return result


# --------------------------------------------------------------------------
# Pass 2


def game_summary(result: dict) -> str:
    """One game's pass-1 answer, compact, for the pass-2 prompt."""
    answer = result.get("answer") or {}
    keep = {
        "id": result["key"],
        "hero": result.get("hero"),
        "place": result.get("place"),
        "patch": result.get("patch"),
        "comp": answer.get("comp"),
        "tribes": answer.get("tribes"),
        "core": answer.get("core"),
        "golden_targets": answer.get("golden_targets"),
        "committed_turn": answer.get("committed_turn"),
        "pivots": answer.get("pivots"),
        "level_turns": answer.get("level_turns"),
        "key_decisions": answer.get("key_decisions"),
        "went_well": answer.get("went_well"),
        "went_badly": answer.get("went_badly"),
        "confidence": answer.get("confidence"),
        "notes": answer.get("notes"),
        "lessons": answer.get("lessons"),
    }
    return json.dumps(keep, ensure_ascii=False)


def comps_header() -> str:
    """comps.toml's own header, as words, so pass 2 follows its fields."""
    lines = []
    for line in COMPS_TOML.read_text().splitlines():
        if not line.startswith("#"):
            break
        lines.append(line.lstrip("#").rstrip())
    return "\n".join(lines).strip()


def facts_block(results: list[dict]) -> str:
    """The counts and averages a model should not have to recount."""
    places = [r["place"] for r in results]
    patches = Counter(r["patch"] for r in results)
    rows = [f"games: {len(results)}",
            f"average place: {sum(places) / len(places):.2f}",
            f"top-4: {sum(1 for p in places if p <= 4)} of {len(places)}",
            "patches: " + ", ".join(f"{k} {v}" for k, v in sorted(patches.items()))]
    by_patch = defaultdict(list)
    for r in results:
        by_patch[r["patch"]].append(r["place"])
    for patch, values in sorted(by_patch.items()):
        rows.append(f"  {patch}: {len(values)} games, average place "
                    f"{sum(values) / len(values):.2f}, top-4 "
                    f"{sum(1 for p in values if p <= 4)}")
    try:
        turn_facts, refresh_facts = level_and_refresh(results)
        rows.append("level-up turns (first turn Era was at each tier): "
                    + turn_facts)
        rows.append("refreshes per turn by the turn's tier: " + refresh_facts)
    except (OSError, json.JSONDecodeError) as error:
        rows.append(f"(level and refresh counts failed: {error})")
    pivot_games = [r for r in results if (r.get("answer") or {}).get("pivots")]
    pivot_turns = [p.get("turn") for r in pivot_games
                   for p in (r["answer"].get("pivots") or [])
                   if isinstance(p.get("turn"), int)]
    rows.append(f"games with at least one pivot: {len(pivot_games)} of "
                f"{len(results)}" + (
                    f", pivot turns: {sorted(pivot_turns)}" if pivot_turns else ""))
    return "\n".join(rows)


def level_and_refresh(results: list[dict]) -> tuple[str, str]:
    """Level-up turns and Refreshes per turn, counted from the games."""
    levels = defaultdict(list)
    rolls = defaultdict(int)
    turns_at = defaultdict(int)
    for result in results:
        game = digest.load_json(digest.ACTIONS_DIR / f"{result['key']}.json")
        reached = {}
        for turn in game.get("turns") or []:
            tier = (turn.get("start") or {}).get("tier")
            if isinstance(tier, int):
                turns_at[tier] += 1
            for step in turn.get("actions") or []:
                if step.get("step") == "level" and step.get("tier_after"):
                    reached.setdefault(int(step["tier_after"]), turn.get("turn"))
                if step.get("step") == "roll" and isinstance(tier, int):
                    rolls[tier] += 1
        for tier, at in reached.items():
            if isinstance(at, int):
                levels[tier].append(at)

    def median(values):
        values = sorted(values)
        return values[len(values) // 2] if values else 0

    rows = []
    for tier in sorted(levels):
        values = sorted(levels[tier])
        rows.append(f"tier {tier}: median turn {median(values)} "
                    f"(range {values[0]} to {values[-1]}, {len(values)} games)")
    refresh_rows = []
    for tier in sorted(turns_at):
        if tier > 6:
            continue
        refresh_rows.append(f"tier {tier}: {rolls[tier] / turns_at[tier]:.2f} "
                            f"per turn ({rolls[tier]} in {turns_at[tier]} turns)")
    return "; ".join(rows), "; ".join(refresh_rows)


def pass2_messages(primer: str, facts: str, summaries: list[str],
                   merge: bool = False, partials: list[dict] | None = None):
    system = primer.strip() + "\n\n" + strategy_text() + "\n\n## comps.toml's header\n\n" + comps_header() + \
             "\n\n" + PASS2_INSTRUCTIONS
    if merge:
        body = [
            "The partial analyses of the games follow, one per chunk. Merge "
            "them into one answer in the same shape: the same comps joined "
            "under one name where they are the same comp, one lessons list "
            "with the repeats combined, and the habits written once.",
            "",
            facts,
            "",
        ]
        for i, partial in enumerate(partials or [], 1):
            body.append(f"### Partial analysis {i}")
            body.append(json.dumps(partial, ensure_ascii=False))
            body.append("")
    else:
        body = ["The data block:", "", facts, "",
                f"Now the {len(summaries)} game summaries, one JSON object "
                "per line:", ""]
        body.extend(summaries)
    return [
        {"role": "system", "content": system},
        {"role": "user", "content": "\n".join(body)},
    ]


def pass2(player, log: RunLog, results: list[dict], folder: Path,
          budget: int = PASS2_BUDGET) -> dict:
    """All the pass-1 answers -> the comps, the habits and the lessons."""
    primer = PRIMER.read_text()
    facts = facts_block(results)
    summaries = [game_summary(r) for r in results]

    chunks: list[list[str]] = [[]]
    size = 0
    for summary in summaries:
        if chunks[-1] and size + len(summary) > budget:
            chunks.append([])
            size = 0
        chunks[-1].append(summary)
        size += len(summary) + 1

    ids = [r["key"] for r in results]
    places = {r["key"]: r["place"] for r in results}
    calls = []
    partials = []
    for i, chunk in enumerate(chunks, 1):
        messages = pass2_messages(primer, facts, chunk)
        kind = "pass2" if len(chunks) == 1 else f"pass2-{i}"
        content, reasoning, usage, seconds = ask(
            player, log, messages, kind,
            context={"ids": ids, "places": places, "merge": False})
        calls.append({"kind": kind, "time": now(), "seconds": round(seconds, 2),
                      "messages": messages, "game": None,
                      "response": {"content": content,
                                   "reasoning_content": reasoning,
                                   "usage": usage}})
        answer = parse_json(content)
        if answer is None:
            raise SystemExit(f"{kind}: the answer was not JSON, see "
                             f"{folder / 'calls.jsonl'}")
        partials.append(answer)

    final = partials[0] if len(partials) == 1 else None
    if final is None:
        messages = pass2_messages(primer, facts, [], merge=True,
                                  partials=partials)
        content, reasoning, usage, seconds = ask(
            player, log, messages, "pass2-merge",
            context={"ids": ids, "places": places, "merge": True})
        calls.append({"kind": "pass2-merge", "time": now(),
                      "seconds": round(seconds, 2), "messages": messages,
                      "game": None,
                      "response": {"content": content,
                                   "reasoning_content": reasoning,
                                   "usage": usage}})
        final = parse_json(content)
        if final is None:
            raise SystemExit("pass2-merge: the answer was not JSON, see "
                             f"{folder / 'calls.jsonl'}")

    log.markdown(folder / "pass2.md", "Pass 2: comps, habits and lessons", calls)
    return final


# --------------------------------------------------------------------------
# The pass-2 output files


def toml_string(text) -> str:
    """A TOML basic string (JSON escapes are TOML's)."""
    return json.dumps(str(text), ensure_ascii=True)


def toml_list(values) -> str:
    return "[" + ", ".join(toml_string(v) for v in values) + "]"


def card_ids() -> dict[str, int]:
    """Card name -> dbf_id, from the current export."""
    out: dict[str, int] = {}
    cards, meta = digest.export(digest.NEW_BUILD)
    for card_id, entry in list(cards.items()) + list(meta.items()):
        name = entry.get("name")
        if not name or entry.get("is_golden"):
            continue
        out.setdefault(name, entry.get("dbf_id"))
    return {name: dbf for name, dbf in out.items() if dbf}


def comp_stats(ids: list[str], results_by_key: dict[str, dict]) -> dict:
    """Games, average place, top-4 rate and the patch split of one comp."""
    rows = [results_by_key[i] for i in ids if i in results_by_key]
    places = [r["place"] for r in rows]

    def block(values):
        if not values:
            return None
        return {
            "games": len(values),
            "average_place": round(sum(values) / len(values), 2),
            "top4_rate": round(sum(1 for p in values if p <= 4) / len(values), 2),
        }

    by_patch = {}
    for patch in sorted({r["patch"] for r in rows}):
        by_patch[patch] = block([r["place"] for r in rows if r["patch"] == patch])
    return {
        "games": len(rows),
        "average_place": round(sum(places) / len(places), 2) if places else None,
        "top4_rate": round(sum(1 for p in places if p <= 4) / len(places), 2)
                     if places else None,
        "by_patch": by_patch,
    }


def write_comps_toml(path: Path, final: dict, results_by_key: dict[str, dict],
                     header: str) -> list[str]:
    known = card_ids()
    wanted: list[str] = []
    rows = [
        "# Comps from Era's own games, written by " + MODEL + ".",
        "# Pass 2 of train/llm/era_comps.py; see report.md and",
        "# lessons-from-era.md in the same folder. The numbers per comp are",
        "# counted from the pass-1 files, not written by the model.",
        "#",
        "# The fields follow comps.toml (tribes, core, golden, addons, when",
        "# Era commits), with the counts this run could measure added.",
    ]
    players = Counter(str(r.get("model")) for r in results_by_key.values())
    rows.append("# Answers from: " + ", ".join(f"{k} {v}"
                                              for k, v in sorted(players.items())))
    rows.append("")
    for comp in final.get("comps") or []:
        name = str(comp.get("name") or "unnamed comp")
        games = [str(g) for g in comp.get("games") or []]
        stats = comp_stats(games, results_by_key)
        core = [str(c) for c in comp.get("core") or []]
        golden = [str(c) for c in comp.get("golden") or []]
        addons = [str(c) for c in comp.get("addons") or []]
        wanted.extend(core + golden + addons)
        rows.append("[[comp]]")
        rows.append(f"name = {toml_string(name)}")
        rows.append(f"tribes = {toml_list([str(t) for t in comp.get('tribes') or []])}")
        if comp.get("summary"):
            rows.append(f"summary = {toml_string(comp['summary'])}")
        rows.append(f"core = {toml_list(core)}")
        rows.append(f"golden = {toml_list(golden)}")
        rows.append(f"addons = {toml_list(addons)}")
        if comp.get("commit"):
            rows.append("commit = [{ all = [], note = "
                        + toml_string(comp["commit"]) + " }]")
        if comp.get("plan"):
            rows.append(f"plan = {toml_string(comp['plan'])}")
        rows.append(f"confidence = {toml_string(comp.get('confidence') or 'low')}")
        rows.append(f"games = {stats['games']}")
        if stats["average_place"] is not None:
            rows.append(f"average_place = {stats['average_place']}")
        if stats["top4_rate"] is not None:
            rows.append(f"top4_rate = {stats['top4_rate']}")
        rows.append(f"game_ids = {toml_list(games)}")
        rows.append("")
        for patch, values in (stats["by_patch"] or {}).items():
            if not values:
                continue
            rows.append(f"[comp.by_patch.{toml_string(patch)}]")
            rows.append(f"games = {values['games']}")
            rows.append(f"average_place = {values['average_place']}")
            rows.append(f"top4_rate = {values['top4_rate']}")
            rows.append("")
    rows.append("# Names the comps use, with the game's own dbfId.")
    rows.append("[cards]")
    missing = []
    for name in sorted(set(wanted)):
        dbf = known.get(name)
        if dbf is None:
            missing.append(name)
            continue
        rows.append(f"{toml_string(name)} = {dbf}")
    path.write_text("\n".join(rows) + "\n")
    return missing


def lesson_rows(final: dict, results_by_key: dict[str, dict]) -> tuple[list, list, list]:
    """(top-4 lessons, bottom-4 lessons, mixed), each row with its counts."""
    top, bottom, mixed = [], [], []
    for lesson in final.get("lessons") or []:
        text = str(lesson.get("lesson") or "").strip()
        if not text:
            continue
        ids = [str(g) for g in lesson.get("games") or [] if str(g) in results_by_key]
        places = [results_by_key[i]["place"] for i in ids]
        row = {
            "lesson": text,
            "kind": lesson.get("kind") or "",
            "games": ids,
            "count": len(places),
            "average_place": round(sum(places) / len(places), 2) if places else None,
            "top4": sum(1 for p in places if p <= 4),
            "bottom4": sum(1 for p in places if p > 4),
        }
        if not places:
            row["average_place"] = None
        if row["top4"] > row["bottom4"]:
            top.append(row)
        elif row["bottom4"] > row["top4"]:
            bottom.append(row)
        else:
            mixed.append(row)
    return top, bottom, mixed


def lesson_line(row: dict) -> str:
    counted = f"{row['count']} game" + ("s" if row["count"] != 1 else "")
    if row["average_place"] is not None:
        counted += f", average place {row['average_place']:.2f}"
    counted += f" ({row['top4']} top-4, {row['bottom4']} bottom-4)"
    kind = f" [{row['kind']}]" if row["kind"] else ""
    return f"- {row['lesson']}{kind} ({counted})"


def write_lessons_md(path: Path, final: dict, results: list[dict],
                     results_by_key: dict[str, dict]) -> None:
    top, bottom, mixed = lesson_rows(final, results_by_key)
    places = [r["place"] for r in results]
    patches = Counter(r["patch"] for r in results)
    rows = [
        "# Lessons learned from Era's games",
        "",
        f"{len(results)} games, average place "
        f"{sum(places) / len(places):.2f}, top-4 "
        f"{sum(1 for p in places if p <= 4)} of {len(places)}; patches: "
        + ", ".join(f"{k} {v}" for k, v in sorted(patches.items())) + ".",
        "",
        f"Merged by {MODEL} from the per-game lessons of pass 1 "
        f"({now()[:10]}). Each line says how many games it came from and the "
        "average place of those games.",
        "",
        "## Lessons from the top-4 games",
        "",
    ]
    if top:
        rows.extend(lesson_line(row) for row in top)
    else:
        rows.append("- (none)")
    rows.extend(["", "## Lessons from the bottom-4 games", ""])
    if bottom:
        rows.extend(lesson_line(row) for row in bottom)
    else:
        rows.append("- (none)")
    if mixed:
        rows.extend(["", "## From as many top-4 as bottom-4 games", ""])
        rows.extend(lesson_line(row) for row in mixed)
    path.write_text("\n".join(rows) + "\n")


def write_report_md(path: Path, final: dict, results: list[dict],
                    results_by_key: dict[str, dict], name: str,
                    missing_cards: list[str], log: RunLog,
                    skipped: dict[str, list[str]]) -> None:
    places = [r["place"] for r in results]
    patches = Counter(r["patch"] for r in results)
    rows = [
        f"# Era's comps and habits: {name}",
        "",
        f"{len(results)} of Era's games, {now()[:10]}, model {MODEL}.",
        f"Average place {sum(places) / len(places):.2f}, top-4 "
        f"{sum(1 for p in places if p <= 4)} of {len(places)}.",
        "Patches: " + ", ".join(f"{k} {v}" for k, v in sorted(patches.items())) + ".",
        "",
        "## The comps",
        "",
        "| comp | tribes | games | average place | top-4 | 37.0 | 37.0.2 "
        "| confidence |",
        "|---|---|---|---|---|---|---|---|",
    ]
    assigned: set[str] = set()
    for comp in final.get("comps") or []:
        games = [str(g) for g in comp.get("games") or []]
        assigned.update(games)
        stats = comp_stats(games, results_by_key)
        by_patch = stats["by_patch"] or {}

        def cell(patch):
            values = by_patch.get(patch)
            if not values:
                return "-"
            return (f"{values['games']} games, {values['average_place']:.2f}, "
                    f"top-4 {values['top4_rate']:.2f}")

        average = ("-" if stats["average_place"] is None
                   else f"{stats['average_place']:.2f}")
        top4 = ("-" if stats["top4_rate"] is None
                else f"{stats['top4_rate']:.2f}")
        tribes = ", ".join(comp.get("tribes") or []) or "any"
        rows.append(
            f"| {comp.get('name')} | {tribes} | {stats['games']} | {average} "
            f"| {top4} | {cell('37.0')} | {cell('37.0.2')} | "
            f"{comp.get('confidence') or 'low'} |"
        )
    rows.append("")

    habits = final.get("habits") or {}
    rows.extend(["## Era's habits", ""])
    for key, title in (("level_up_turns", "Level-up turns"),
                       ("refreshes", "Refreshes by tier"),
                       ("pivots", "Pivots"),
                       ("top4_vs_bottom4", "Top-4 games against bottom-4 games")):
        if habits.get(key):
            rows.append(f"**{title}.** {habits[key]}")
            rows.append("")
    rows.extend(["Counted from the games (not from the model):", "",
                 "```", facts_block(results), "```", ""])

    top, bottom, mixed = lesson_rows(final, results_by_key)
    rows.extend([f"## Lessons learned from Era ({len(top) + len(bottom) + len(mixed)} "
                 "merged)", "",
                 "See lessons-from-era.md for the full list with counts and "
                 "places.", ""])
    for row in (top + bottom + mixed)[:8]:
        rows.append(lesson_line(row))
    rows.append("")

    rows.extend(["## Things worth asking Era", ""])
    for question in final.get("ask_era") or []:
        rows.append(f"- {question}")
    if not final.get("ask_era"):
        rows.append("- (none)")
    rows.append("")

    unassigned = [r for r in results if r["key"] not in assigned]
    if unassigned:
        rows.extend(["## Games no comp claimed", "",
                     ", ".join(f"{r['key']} (place {r['place']})"
                               for r in unassigned), ""])

    if missing_cards:
        rows.extend(["## Names the comps use that the export does not have",
                     "", ", ".join(missing_cards), ""])

    players = Counter(str(r.get("model")) for r in results)
    rows.extend(["## The run", "",
                 "answers from: " + ", ".join(f"{k} {v}"
                                              for k, v in sorted(players.items())),
                 ""])
    for what, keys in skipped.items():
        if keys:
            rows.append(f"- {what}: {len(keys)} ({', '.join(keys[:6])}"
                        + (", ..." if len(keys) > 6 else "") + ")")
    rows.append(f"- calls: {log.calls} (pass 1 and pass 2 together), prompt "
                f"tokens {log.prompt_tokens}, completion tokens "
                f"{log.completion_tokens}")
    rows.append("- per-game answers: game-<source>-g<N>.json and .md")
    rows.append("")
    path.write_text("\n".join(rows))


# --------------------------------------------------------------------------
# The script


def select(args, folder: Path) -> tuple[list[dict], dict[str, list[str]]]:
    skipped = {"unfinished games": [], "other patch": [], "already done": []}
    infos = []
    for path in digest.game_files():
        info = digest.game_info(path)
        if info["unfinished"]:
            skipped["unfinished games"].append(info["key"])
            continue
        if args.only_patch and info["patch"] != args.only_patch:
            skipped["other patch"].append(info["key"])
            continue
        if args.resume and (folder / f"game-{info['key']}.json").exists():
            skipped["already done"].append(info["key"])
            continue
        infos.append(info)
    if args.games:
        infos = infos[:args.games]
    return infos, skipped


def load_results(folder: Path, only_patch: str | None) -> list[dict]:
    results = []
    for path in sorted(folder.glob("game-*.json")):
        try:
            result = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        if not result.get("answer"):
            continue
        if only_patch and result.get("patch") != only_patch:
            continue
        if result.get("unfinished"):
            continue
        results.append(result)
    return results


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        description="DeepSeek reads Era's games and finds the comps.")
    parser.add_argument("--name", default=None,
                        help="run folder under train/runs (default comps-<date>)")
    parser.add_argument("--games", type=int, default=0,
                        help="how many games to analyse in pass 1")
    parser.add_argument("--only-patch", choices=("37.0", "37.0.2"), default=None,
                        help="only games from this patch")
    parser.add_argument("--player", choices=("fake", "deepseek"),
                        default="deepseek", help="fake: offline canned answers")
    parser.add_argument("--resume", action="store_true",
                        help="skip games whose JSON already exists")
    parser.add_argument("--pass2-only", action="store_true",
                        help="skip pass 1 and merge the pass-1 files already there")
    parser.add_argument("--parallel", type=int, default=8,
                        help="games in parallel in pass 1 (default 8)")
    parser.add_argument("--pass2-budget", type=int, default=PASS2_BUDGET,
                        help="characters of game summaries in one pass-2 call "
                             "before it is split into chunks (default "
                             f"{PASS2_BUDGET})")
    args = parser.parse_args(argv)

    name = args.name or f"comps-{datetime.now().strftime('%Y-%m-%d')}"
    folder = RUNS / name
    folder.mkdir(parents=True, exist_ok=True)
    log = RunLog(folder)

    if args.player == "fake":
        player = FakePlayer(log)
    else:
        key = os.environ.get(KEY_VAR)
        if not key:
            print(f"{KEY_VAR} is not set: pass --player fake to test offline, "
                  "or export the key.", file=sys.stderr)
            return 2
        player = DeepSeekPlayer(key, log)

    infos, skipped = select(args, folder)
    if args.pass2_only:
        print(f"run {name}: pass 2 only, from the pass-1 files in {folder}")
    else:
        print(f"run {name}: {len(infos)} games to analyse, player {player.kind}")

    stop = threading.Event()

    def work(info):
        if stop.is_set():
            return None
        try:
            result = pass1_game(info, player, log, folder)
        except StopGame as error:
            stop.set()
            log.note(f"stopping: {error}")
            print(f"stopping: {error}")
            return None
        except (OSError, json.JSONDecodeError) as error:
            log.note(f"{info['key']}: failed ({error})")
            print(f"{info['key']}: failed ({error})")
            return None
        answer = result.get("answer") or {}
        print(f"  {info['key']:<48} place {result['place']} "
              f"{answer.get('comp') or '(no answer)'} "
              f"({result['seconds']} s)")
        return result

    if not args.pass2_only and infos:
        with concurrent.futures.ThreadPoolExecutor(
                max_workers=max(1, args.parallel)) as pool:
            list(pool.map(work, infos))
        if stop.is_set():
            print("stopped early; rerun with --resume to continue")
            return 1

    results = load_results(folder, args.only_patch)
    players = Counter(str(r.get("model")) for r in results)
    if player.kind != "fake" and players.get("fake"):
        print(f"warning: {players['fake']} of these pass-1 answers are FAKE "
              f"(offline test answers); they are merged like any other. Use a "
              f"fresh --name for a real analysis.")
    print(f"pass 2 over {len(results)} games")
    if not results:
        print("no pass-1 answers to merge: nothing written", file=sys.stderr)
        return 1

    final = pass2(player, log, results, folder, budget=args.pass2_budget)
    results_by_key = {r["key"]: r for r in results}
    missing = write_comps_toml(folder / "comps-era.toml", final, results_by_key,
                              comps_header())
    write_lessons_md(folder / "lessons-from-era.md", final, results, results_by_key)
    write_report_md(folder / "report.md", final, results, results_by_key, name,
                    missing, log, skipped)

    print(f"wrote {folder}/comps-era.toml, report.md, lessons-from-era.md")
    print(f"calls: {log.calls}, prompt tokens {log.prompt_tokens}, "
          f"completion tokens {log.completion_tokens}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
