"""PPO self-play for the Battlegrounds lobby (5 of the 10 tribes each game).

    .venv/bin/python train/ppo.py --name stage2-smoke --minutes 5
    .venv/bin/python train/ppo.py --name run2 --resume runs/run2/ckpt.pt
    .venv/bin/python train/ppo.py --name attn1 --model attn

--model picks the network (model.py): mlp (the default), attn or pool.
--opp-boards says how attn sees opponent boards (next by default). Both are
saved in the checkpoint, and --resume builds what the checkpoint holds.
--init-from starts a new run from another checkpoint's weights, every
parameter whose name and shape match, with a fresh optimizer:

    .venv/bin/python train/ppo.py --name attn2 --model attn --opp-boards next \
        --init-from runs/attn1/best.pt

Each game seats learners (this network, trained), snapshots (older copies of
it), standard curve bots, rush curve bots, random bots and real seats
(--lineup has all six counts; a real seat plays a board of the current turn
from Era's games, --real-boards, and never shops). Reward is placement:
+1 for 1st down to -1 for 8th (plus optional --first-bonus and
--second-bonus for 1st and 2nd), given when the seat is knocked out or the
game ends, minus a tiny cost per board move (--action-cost, --cost-kinds) so
it doesn't shuffle its board for free. --fight-reward W adds +W to a learner
turn end when the fight that follows is won, 0 for a tie and -W for a loss
(default 0: no fight term). --fight-score W instead adds W times the fight's
(uncapped damage dealt - uncapped damage taken) / 48 at the same turn end, so
a heavy loss differs from a light one; --fight-score-from skips early turns
and --fight-score-fade lowers the weight to 0 by the run's planned end.
--win-score W adds +W for a fight won, W/2 for a tie and 0 for a loss at the
same turn end, from --win-score-from on (default 9), with --win-score-fade the
same fade; a knockout is a loss.

Everything lands in runs/<name>/: metrics.csv (one row per eval), a
TensorBoard log, ckpt.pt (latest) and best.pt (best eval: vs7 real when the
run has real boards, else against the curve bots).
"""

import argparse
import copy
import csv
import random
import sys
import time
import tomllib
from pathlib import Path

import numpy as np
import torch
import torch.nn.functional as F
from torch.utils.tensorboard import SummaryWriter

import bgpy

# --lag-in-obs changes the observation layout (one extra own scalar), and
# model reads bgpy's layout constants when it is imported: settle the flag
# before that import. Off restores the old layout exactly.
# allow_abbrev=False: with prefix matching, any importer's option that starts
# with --lag (say --lag 2) would switch the layout without a word.
_pre = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
_pre.add_argument("--lag-in-obs", action="store_true")
LAG_IN_OBS = _pre.parse_known_args()[0].lag_in_obs
bgpy.set_lag_in_obs(LAG_IN_OBS)

import card_text
import comp_specs
import real_boards
from model import (build, ck_model, load as load_model, masked, model_args,
                   model_opts, names_at_rev as toml_names_at_rev, opts_text,
                   set_recompute, text_gates, warm_start)

sys.path.insert(0, str(Path(__file__).resolve().parent / "evaltools"))
import buy_prior as bp

LEARNER, SNAPSHOT = bgpy.LEARNER, bgpy.SNAPSHOT
P = bgpy.PLAYERS
KINDS = [name for name, _ in bgpy.ACTION_KINDS]
KIND_OF = torch.tensor([k for k, (_, n) in enumerate(bgpy.ACTION_KINDS) for _ in range(n)])
# --demo-kind-weight groups the 18 action kinds into the ten ways a turn moves.
# Activate is using a board minion's own card, so it joins play; discard,
# target and every pick answer an open prompt, so they join choose.
DEMO_KIND_GROUPS = ("buy", "sell", "play", "roll", "level", "freeze",
                    "hero_power", "move", "choose", "end")
_KIND_GROUP = {
    "buy": "buy", "buy_spell": "buy", "sell": "sell",
    "activate": "play", "play": "play",
    "target": "choose", "target_shop": "choose", "choose": "choose",
    "discard": "choose", "move": "move", "roll": "roll", "freeze": "freeze",
    "level": "level", "end": "end",
    "hero_power": "hero_power", "hero_power2": "hero_power",
    "choose_trinket": "choose", "dark_gift": "choose",
}
if set(_KIND_GROUP) != set(KINDS):
    raise RuntimeError(f"demo kind groups do not cover the action kinds {KINDS}")
DEMO_KIND_OF_KIND = torch.tensor(
    [DEMO_KIND_GROUPS.index(_KIND_GROUP[name]) for name in KINDS])
DEMO_KIND_OF = DEMO_KIND_OF_KIND[KIND_OF]
END_TURN = bgpy.END_TURN
SHOP_START = bgpy.SHOP_START
ACTIONS_LEFT = bgpy.SCALAR_ACTIONS_LEFT
WON_LAST_COMBAT = bgpy.SCALAR_WON_LAST_COMBAT
LOST_LAST_COMBAT = bgpy.SCALAR_LOST_LAST_COMBAT
TURN = bgpy.SCALAR_TURN

TRIBE_NAMES = ("beast", "mech", "dragon", "murloc", "demon", "pirate",
               "elemental", "undead", "quilboar", "aberration")
REPORT_GROUPS = TRIBE_NAMES + ("mixed",)
EVAL_KEYS = [
    "place_vs_std", "place_vs_rush", "place_4v4_std", "place_4v4_rush", "place_4v4_mix",
    "place_vs_snapshot",
    "turn_t3", "turn_t4", "turn_t5",
    "curve_econ_rush", "curve_rush", "curve_standard", "curve_slow",
    *[f"tribe_{t}" for t in REPORT_GROUPS],
    *[f"place_{t}" for t in REPORT_GROUPS],
    # last, so rows appended to a metrics.csv from before real seats stay
    # aligned with its header (the extra value just trails the old columns)
    "place_vs_real",
    # the wheels-off comp line, after every older column for the same reason
    "comp_active_share", "comp_end_share", "shell_rate", "first_share",
]
# Columns of bgpy's tribe counts: the ten tribes in bgcore's `Tribe::EVERY`
# order, then neutral.
TRIBE_COLS = {t: k for k, t in enumerate(TRIBE_NAMES)}
TRAIN_KEYS = [
    "entropy", "approx_kl", "clipfrac", "value_loss", "explained_var", "steps_per_s",
    "actions_per_turn", "tier_t6", "tier_t10", "health_end",
    "aux_loss", "wheels_b", "wheels_lambda", "comp_active",
]
# Appended after top_buys, so rows in an older metrics.csv keep their columns.
EXTRA_KEYS = ["entropy_plain", "moves_per_turn", "fight_reward_per_turn",
              "fight_score_per_turn", "win_score_per_turn"]
# Appended after every older eval column, only when --demo is on.
DEMO_KEYS = ["demo_top1", "demo_top1_buy", "demo_top1_train"]
# Appended after the exact columns when --demo-kind-weight is on.
DEMO_KIND_KEYS = ["demo_kind_top1", "demo_kind_top1_train"]
DEMO_EVAL_BATCH = 4096
# The BASE the demonstration dataset was built against, for the startup kind
# agreement. Its absence only skips that reference, never a run.
DEMO_BASE_CKPT = Path(__file__).resolve().parent / "runs" / "r1113-a1" / "best.pt"
# The buy prior's slots: the 7 Tavern minions and the Tavern spell, matching
# buy_prior.py's offer order.
BUY_PRIOR_KEYS = ["buy_prior_top1"]
BUY0 = sum(n for _, n in bgpy.ACTION_KINDS[:KINDS.index("buy")])
BUY_ACTIONS = list(range(BUY0, BUY0 + bgpy.SHOP + 1))
SHOP_COLS = [bgpy.SHOP_START + i * bgpy.SLOT_FEATURES for i in range(bgpy.SHOP)] + [
    bgpy.SHOP_SPELL_START]
BOARD_COLS = [bgpy.BOARD_START + i * bgpy.SLOT_FEATURES for i in range(bgpy.BOARD)]
# The startup reference plays this many standard-bot eval games with BASE.
BASE_BUY_PRIOR_GAMES = 128


def args():
    a = argparse.ArgumentParser()
    a.add_argument("--name", required=True)
    a.add_argument("--minutes", type=float, default=10)
    a.add_argument("--resume")
    model_args(a)
    a.add_argument("--compile", action=argparse.BooleanOptionalAction,
                   help="torch.compile the attention layers (default: on for attn)")
    a.add_argument("--init-from", help="checkpoint to take matching weights from, for a new run")
    a.add_argument("--init-cards-rev",
                   help="git revision whose cards.toml, heroes.toml and trinkets.toml an "
                        "--init-from checkpoint was trained on, for checkpoints saved before "
                        "they stored their table names")
    a.add_argument("--warmup", type=int, default=0, help="updates of linear lr warmup")
    a.add_argument("--value-warmup", type=int, default=0,
                   help="first updates that train only the value loss (no policy or entropy "
                        "loss): for an --init-from network whose value head never learned, "
                        "such as imitate.py's, so its first advantages aren't noise")
    a.add_argument("--seed", type=int, default=0)
    a.add_argument("--old-boards", choices=["keep", "drop"], default=None,
                   help="what an opponent's last-seen board is worth in the observation: keep "
                        "writes it however old it is (the observation before 2026-11-06), drop "
                        "writes it only for the opponent fought last turn and for the one faced "
                        "now when their board is at most 2 turns old (docs/notes/era-strategy.md: "
                        "Era judges a foe from the leaderboard, not old boards). Same observation "
                        "size either way, so a run can switch mid-way (default keep, or the "
                        "checkpoint's own on --resume)")
    a.add_argument("--envs", type=int, default=1024)
    a.add_argument("--overlay-comps", default=None,
                   help="comp file for the overlay during training; absent means no overlay")
    a.add_argument("--overlay-bonus", type=float, default=8.0,
                   help="B, the comp payoff bonus")
    a.add_argument("--scenario-comps", default=None,
                   help="semicolon-separated comp names (from --overlay-comps) that scenario "
                        "games start with; absent means no scenario games")
    a.add_argument("--scenario-share", type=float, default=0.0,
                   help="fraction of training games that start as scenario games (0: off)")
    a.add_argument("--scenario-turns", default="7,11",
                   help="MIN,MAX turn a scenario game starts on, drawn uniformly")
    a.add_argument("--era-starts", default=None,
                   help="start some games from Era's logged positions in this action folder")
    a.add_argument("--era-share", type=float, default=0.0,
                   help="fraction of training games that start from Era's positions (0: off)")
    a.add_argument("--era-turns", default="7,11",
                   help="MIN,MAX turn an Era start begins on (default 7,11)")
    a.add_argument("--era-start-lag", type=int, default=None,
                   help="fixed lag for every late-start game; normal games still draw from "
                        "--real-lag-mix. Absent: a late start draws like a normal game")
    a.add_argument("--wheels-full", type=int, default=0,
                   help="games with the overlay at full strength (0 with --wheels-end 0: no fade)")
    a.add_argument("--wheels-end", type=int, default=0,
                   help="games where the overlay has faded to none; between the two, B and lambda "
                        "fall linearly and forced moves stop at the end")
    a.add_argument("--aux-weight", type=float, default=1.0,
                   help="lambda, the weight of the overlay-choice cross-entropy at full strength")
    a.add_argument("--steps", type=int, default=64, help="steps per env per rollout")
    a.add_argument("--lineup", default="2,2,0,0,0,4",
                   help="learner,snapshot,standard,rush,random,real seats")
    a.add_argument("--learner-heroes", default=None,
                   help="comma-separated hero names every learner seat is given instead of "
                        "the random hero deal (e.g. \"Trade Prince Gallywix\"); no other seat "
                        "gets one of them. Empty keeps the normal deal")
    a.add_argument("--real-boards", default=None,
                   help="folder of real boards for real seats (docs/real-boards.md); "
                        "default boards/real when the lineup has real seats. Given without "
                        "real seats it still adds the vs7 real eval")
    a.add_argument("--lr", type=float, default=2.5e-4)
    a.add_argument("--epochs", type=int, default=3)
    a.add_argument("--minibatch", type=int, default=8192)
    a.add_argument("--micro-batch", type=int,
                   help="run each minibatch through the network in pieces of this size and add "
                        "up the gradients: the same update, less GPU memory")
    a.add_argument("--vram-gb", type=float,
                   help="cap this process's GPU memory (GB) so the desktop keeps the rest; "
                        "pair with a smaller --minibatch (4096 halves the training peak at the "
                        "same samples/s; --micro-batch keeps the update the same)")
    a.add_argument("--no-recompute", action="store_true",
                   help="attn: keep each layer's activations instead of recomputing them in the "
                        "backward pass: the same update, about a third less learning time, more "
                        "GPU memory (bgt's night strength)")
    a.add_argument("--card-text", action="store_true",
                   help="give every card's id embedding a vector read from its printed text "
                        "(train/card_text.py); the vocabulary and token table are saved in the "
                        "checkpoint, and --resume keeps the run's own")
    a.add_argument("--power-text", action="store_true",
                   help="add the printed text of every hero power and trinket to its id "
                        "embedding, through a gate that starts at 0; shares --card-text's word "
                        "table, and the rows and gates are saved in the checkpoint")
    a.add_argument("--card-form", action="store_true",
                   help="with --card-text: also add every card's fixed effect form "
                        "(data/card_forms.json), saved in the checkpoint like the text")
    a.add_argument("--obs-fp16", action="store_true",
                   help="store the rollout's observations in float16 (half the buffer) and widen "
                        "them to float32 just before the network sees them; exact, because every "
                        "observation is a whole number below 2048 (checked on 32,768 real ones "
                        "2026-11-06: largest 472, none changed)")
    a.add_argument("--clip", type=float, default=0.2)
    a.add_argument("--ent", type=float, default=0.01)
    a.add_argument("--entropy-moves-as-one", action="store_true",
                   help="count all Move actions as one entropy atom (their summed probability) "
                        "instead of one atom per move; moves stay legal and sampled")
    a.add_argument("--vf", type=float, default=0.5)
    a.add_argument("--gamma", type=float, default=0.99, help="discount per turn, not per action")
    a.add_argument("--lam", type=float, default=0.95, help="per turn, like --gamma")
    a.add_argument("--action-cost", type=float, default=0.002,
                   help="reward taken off each action of --cost-kinds, so it doesn't dither")
    a.add_argument("--fight-reward", type=float, default=0.0,
                   help="+W when the fight after a learner turn is won, -W when lost, 0 for "
                        "a tie; a knockout is a loss and 1st place is a win")
    a.add_argument("--fight-score", type=float, default=0.0,
                   help="W: add W * (uncapped damage dealt - uncapped damage taken) / 48 at a "
                        "learner turn end for the fight that follows it, before the rules' "
                        "damage cap (default 0: off)")
    a.add_argument("--fight-score-from", type=int, default=1,
                   help="first turn whose fight pays --fight-score (default 1: every turn)")
    a.add_argument("--fight-score-fade", type=float, default=0.0,
                   help="fraction of the run over which --fight-score falls linearly to 0 at "
                        "the run's planned end; 0 keeps it at full weight (default 0)")
    a.add_argument("--win-score", type=float, default=0.0,
                   help="W: add W for a won fight, W/2 for a tie and 0 for a loss at a learner "
                        "turn end; a knockout is a loss (default 0: off)")
    a.add_argument("--win-score-from", type=int, default=9,
                   help="first turn whose fight pays --win-score (default 9)")
    a.add_argument("--win-score-fade", type=float, default=0.0,
                   help="fraction of the run over which --win-score falls linearly to 0 at "
                        "the run's planned end; 0 keeps it at full weight (default 0)")
    a.add_argument("--demo", help="demonstration dataset from train/evaltools/demo_dataset.py "
                                  "(Era's observations, legal masks and actions)")
    a.add_argument("--demo-weight", type=float, default=0.0,
                   help="W: each update adds W times the cross-entropy between the network's "
                        "masked policy and Era's action on a fresh train-split demonstration "
                        "minibatch of the training minibatch's size (default 0: off)")
    a.add_argument("--demo-fade", type=float, default=0.0,
                   help="fraction of the run over which --demo-weight falls linearly to 0 at "
                        "the run's planned end; 0 keeps it at full weight (default 0)")
    a.add_argument("--demo-kind-weight", type=float, default=0.0,
                   help="W: each update adds W times the cross-entropy between the network's "
                        "masked policy summed over the legal actions of each grouped action kind "
                        "and Era's kind, on the same fresh train-split demonstration minibatch "
                        "(needs --demo; may be used with or without --demo-weight; default 0: off)")
    a.add_argument("--buy-prior", help="BuyPrior checkpoint from train/evaltools/buy_prior.py "
                                       "(which Tavern cards strong players buy)")
    a.add_argument("--buy-prior-weight", type=float, default=0.0,
                   help="W: each update adds W times the cross-entropy from the buy model's "
                        "softmax over the legal buy slots to the network's own softmax over the "
                        "same slots (needs --buy-prior; default 0: off)")
    a.add_argument("--buy-prior-fade", type=float, default=0.0,
                   help="fraction of the run over which --buy-prior-weight falls linearly to 0 "
                        "at the run's planned end; 0 keeps it at full weight (default 0)")
    a.add_argument("--force-turn2-level", action="store_true",
                   help="a tier 1 seat that can level on turn 2 must level (learner and snapshots)")
    a.add_argument("--force-dark-gift", action="store_true",
                   help="on turns 3, 4 and 5 every learner and snapshot seat presses Dark "
                        "Discovery first thing when it can, in training and test games")
    a.add_argument("--real-lobby-filter", action="store_true",
                   help="a real seat draws only boards whose minions fit this game's lobby")
    a.add_argument("--real-lag-mix", default=None,
                   help="weighted lags each training game draws its real seats' lag from at "
                        "game start, as LAG:WEIGHT pairs, e.g. 0:1,1:1,2:1. Each game keeps its "
                        "lag; absent means 0. The in-run vs7 real eval stays at lag 0, and a "
                        "separate mixed-lag eval reports the share and place per lag")
    a.add_argument("--real-lag-from", type=int, default=1,
                   help="first turn the real seats' lag applies on; before it they draw their "
                        "own turn as at lag 0 (default 1: every turn)")
    a.add_argument("--lag-in-obs", action="store_true",
                   help="write the game's lag as one extra own scalar, so the value head can "
                        "tell easy games from hard ones. Off is the old observation exactly")
    a.add_argument("--first-bonus", type=float, default=0.0,
                   help="extra reward for 1st place, added to the linear placement reward")
    a.add_argument("--second-bonus", type=float, default=0.0,
                   help="extra reward for 2nd place, added to the linear placement reward")
    a.add_argument("--low-penalty", type=float, default=0.0,
                   help="reward taken off at each turn end per tier below the standard curve")
    a.add_argument("--low-penalty-fade", type=float, default=240,
                   help="run minutes over which --low-penalty fades linearly to 0")
    a.add_argument("--deadline-penalty", action="store_true",
                   help="Era's levelling deadlines (2026-11-02): at each turn end, a free seat "
                        "short of tier 2 from turn 2 pays --deadline-turn2 per missing tier, short "
                        "of the --deadline-curve tier pays --deadline-later")
    a.add_argument("--deadline-curve", default="0,0,0,0,0,3,3,4",
                   help="tier needed at the end of turns 1, 2, ... for --deadline-later, the last "
                        "value holding for every later turn (0: none). Default: tier 3 by turn 6, "
                        "tier 4 by turn 8; Era's own curve is 0,0,0,0,3,3,4,5")
    a.add_argument("--deadline-turn2", type=float, default=0.30)
    a.add_argument("--deadline-turn2-floor", type=float, default=0.10,
                   help="what the turn 2 part fades to and keeps: levelling on turn 2 is "
                        "right most of the time, so a slight penalty stays")
    a.add_argument("--deadline-later", type=float, default=0.15)
    a.add_argument("--deadline-full", type=float, default=180,
                   help="run minutes at full strength, then it fades linearly")
    a.add_argument("--deadline-end", type=float, default=360,
                   help="run minute where the deadline penalty reaches 0")
    a.add_argument("--cost-kinds", default="move",
                   help="action kinds that pay --action-cost, comma separated, or all")
    a.add_argument("--snapshot-every", type=int, default=250, help="updates")
    a.add_argument("--keep-every", type=float, default=30, help="minutes between weights-only copies in keep/")
    a.add_argument("--stop-flat", type=float, default=0,
                   help="stop after this many minutes without gains (0: never)")
    a.add_argument("--snapshot-pool", type=int, default=20)
    a.add_argument("--eval-every", type=int, default=20, help="updates")
    a.add_argument("--eval-games", type=int, default=512)
    cfg = a.parse_args()
    if cfg.fight_score and cfg.value_warmup:
        a.error("--fight-score cannot be used with --value-warmup: a reward change must not "
                "start with a critic-only warm-up (the policy moves through the shared trunk)")
    if not 0.0 <= cfg.fight_score_fade <= 1.0:
        a.error("--fight-score-fade must be between 0 and 1")
    if not 0.0 <= cfg.win_score_fade <= 1.0:
        a.error("--win-score-fade must be between 0 and 1")
    if cfg.demo_weight < 0.0:
        a.error("--demo-weight cannot be negative")
    if cfg.demo_weight and not cfg.demo:
        a.error("--demo-weight needs --demo PATH")
    if cfg.demo_kind_weight < 0.0:
        a.error("--demo-kind-weight cannot be negative")
    if cfg.demo_kind_weight and not cfg.demo:
        a.error("--demo-kind-weight needs --demo PATH")
    if not 0.0 <= cfg.demo_fade <= 1.0:
        a.error("--demo-fade must be between 0 and 1")
    if cfg.buy_prior_weight < 0.0:
        a.error("--buy-prior-weight cannot be negative")
    if cfg.buy_prior_weight and not cfg.buy_prior:
        a.error("--buy-prior-weight needs --buy-prior PATH")
    if not 0.0 <= cfg.buy_prior_fade <= 1.0:
        a.error("--buy-prior-fade must be between 0 and 1")
    return cfg


def reward(place, first_bonus=0.0, second_bonus=0.0):
    value = (4.5 - place) / 3.5
    if place == 1 and first_bonus:
        return value + first_bonus
    if place == 2 and second_bonus:
        return value + second_bonus
    return value


def fight_outcome(won, lost):
    """+1 for a won fight, -1 for a lost one, 0 for a tie."""
    if won > 0.5:
        return 1
    if lost > 0.5:
        return -1
    return 0


def place_fight_outcome(place):
    """1st place is a win; any other placement is a loss."""
    return 1 if place == 1 else -1


def win_score_outcome(won, lost):
    """1 for a won fight, 0.5 for a tie, 0 for a loss."""
    if won > 0.5:
        return 1.0
    if lost > 0.5:
        return 0.0
    return 0.5


def place_win_score_outcome(place):
    """1st place is a win; any other placement (a knockout) pays 0."""
    return 1.0 if place == 1 else 0.0


def credit_fight(rews, pending, env, seat, outcome, weight):
    """Credit weight * outcome to the pending turn-end step for (env, seat).

    Returns the term added, or None when no turn is pending there. The
    pending index is cleared either way, so one fight is credited once.
    """
    step = int(pending[env, seat])
    if step < 0:
        return None
    pending[env, seat] = -1
    term = weight * outcome
    rews[step, env] += term
    return term


# The divisor of the graded fight score: a 48-damage swing is worth one
# placement point (1st to 8th is 2), so the term stays small beside placement.
FIGHT_SCORE_SCALE = 48.0


def credit_fight_score(rews, pending, pending_turn, env, seat, dealt, taken, weight,
                       from_turn=1):
    """Credit weight * (uncapped dealt - uncapped taken) / 48 at the turn end.

    `dealt` and `taken` are the engine's raw damage before the rules' cap for
    the fight after that turn. A tie is 0 (both are 0), and so is a turn
    before `from_turn`. Returns the term added, or None when no turn is
    pending there; the pending index is cleared either way.
    """
    step = int(pending[env, seat])
    if step < 0:
        return None
    pending[env, seat] = -1
    if int(pending_turn[env, seat]) < from_turn:
        return 0.0
    term = weight * (float(dealt) - float(taken)) / FIGHT_SCORE_SCALE
    rews[step, env] += term
    return term


def credit_win_score(rews, pending, pending_turn, env, seat, outcome, weight,
                     from_turn=9):
    """Credit weight * outcome at the turn-end step for (env, seat).

    `outcome` is 1 for a won fight, 0.5 for a tie and 0 for a loss. A turn
    before `from_turn` pays 0. Returns the term added, or None when no turn is
    pending there; the pending index is cleared either way.
    """
    step = int(pending[env, seat])
    if step < 0:
        return None
    pending[env, seat] = -1
    if int(pending_turn[env, seat]) < from_turn:
        return 0.0
    term = weight * float(outcome)
    rews[step, env] += term
    return term


def fight_score_fade_scale(run_min, run_start, run_end, fade):
    """The share of --fight-score still paid at `run_min`.

    1 until the last `fade` fraction of the run (from `run_start` to
    `run_end`), then linearly to 0 at `run_end`. fade 0 is no fade, and a
    run already past its end pays nothing.
    """
    if fade <= 0 or run_end <= run_start:
        return 1.0
    fade_start = run_end - fade * (run_end - run_start)
    if run_min <= fade_start:
        return 1.0
    if run_min >= run_end:
        return 0.0
    return (run_end - run_min) / (run_end - fade_start)


def win_score_fade_scale(run_min, run_start, run_end, fade):
    """The share of --win-score still paid at `run_min` (same shape as the
    fight score's fade)."""
    return fight_score_fade_scale(run_min, run_start, run_end, fade)


# The demonstration loss fades exactly like the fight and win scores.
demo_fade_scale = fight_score_fade_scale


class DemoData:
    """The Build A demonstration dataset: observations, legal masks, Era's
    actions and the game split. Rows are held on the CPU; the training and
    eval loops move only the rows they use."""

    def __init__(self, obs, mask, action, split, source="<arrays>"):
        if torch.is_tensor(obs):
            obs = obs.detach().cpu().numpy()
        if torch.is_tensor(mask):
            mask = mask.detach().cpu().numpy()
        if torch.is_tensor(action):
            action = action.detach().cpu().numpy()
        if torch.is_tensor(split):
            split = split.detach().cpu().numpy()
        self.obs = torch.as_tensor(np.ascontiguousarray(obs, dtype=np.float32))
        self.mask = torch.as_tensor(np.ascontiguousarray(mask, dtype=bool))
        self.action = torch.as_tensor(np.ascontiguousarray(action, dtype=np.int64))
        self.kind = DEMO_KIND_OF[self.action]
        split = np.asarray(split)
        if self.obs.ndim != 2:
            raise SystemExit(f"{source}: obs must be 2-D, got shape {tuple(self.obs.shape)}")
        n = len(self.obs)
        if self.mask.shape != (n, bgpy.N_ACTIONS):
            raise SystemExit(f"{source}: mask must be ({n}, {bgpy.N_ACTIONS}), "
                             f"got {tuple(self.mask.shape)}")
        if self.action.shape != (n,):
            raise SystemExit(f"{source}: action must be ({n},), got {tuple(self.action.shape)}")
        if split.shape != (n,):
            raise SystemExit(f"{source}: split must be ({n},), got {split.shape}")
        train = np.flatnonzero(split == "train")
        heldout = np.flatnonzero(split == "heldout")
        if not len(train) or not len(heldout):
            raise SystemExit(f"{source}: needs train and heldout rows, has "
                             f"{len(train)} train and {len(heldout)} heldout")
        self.train = torch.as_tensor(train, dtype=torch.long)
        self.heldout = torch.as_tensor(heldout, dtype=torch.long)
        # A fixed train sample the size of the held-out split, to watch
        # overfitting through the same number as the held-out agreement.
        rng = np.random.default_rng(0)
        size = min(len(train), len(heldout))
        pick = np.sort(rng.choice(len(train), size=size, replace=False))
        self.train_fixed = self.train[torch.as_tensor(pick, dtype=torch.long)]
        buy = KINDS.index("buy")
        lo = sum(count for _, count in bgpy.ACTION_KINDS[:buy])
        hi = lo + bgpy.ACTION_KINDS[buy][1]
        self.is_buy = (self.action >= lo) & (self.action < hi)


def load_demo(path, obs_size):
    """Read a demonstration npz and refuse an observation layout this network
    does not take."""
    path = Path(path)
    try:
        npz = np.load(path, allow_pickle=False)
    except (OSError, ValueError) as e:
        raise SystemExit(f"--demo {path}: cannot read: {e}")
    with npz as z:
        missing = [k for k in ("obs", "mask", "action", "split") if k not in z.files]
        if missing:
            raise SystemExit(f"--demo {path}: missing {', '.join(missing)}")
        obs, mask, action, split = z["obs"], z["mask"], z["action"], z["split"]
    if obs.ndim != 2:
        raise SystemExit(f"--demo {path}: obs must be 2-D, got shape {obs.shape}")
    if obs.shape[1] != obs_size:
        raise SystemExit(
            f"--demo {path}: observations are {obs.shape[1]} wide, this network takes "
            f"{obs_size}. The dataset must match the network's observation "
            "(old_boards='keep' and no --lag-in-obs); rebuild it with "
            "train/evaltools/demo_dataset.py")
    return DemoData(obs, mask, action, split, source=str(path))


def demo_cross_entropy(net, obs, mask, action):
    """The masked policy's cross-entropy to Era's action, averaged over rows."""
    logits, _ = net(obs)
    return F.cross_entropy(masked(logits, mask), action)


def add_demo_loss(loss, net, obs, mask, action, weight):
    """Add the demonstration term to a PPO loss. Weight 0 returns `loss`
    itself, so today's loss is unchanged exactly."""
    if weight == 0.0:
        return loss
    return loss + weight * demo_cross_entropy(net, obs, mask, action)


def demo_kind_probs(logits, mask):
    """The masked policy summed over the legal actions of each grouped kind:
    one probability per row and DEMO_KIND_GROUPS entry. Illegal actions are
    zero, so the row sums to 1 over the legal kinds."""
    probs = torch.softmax(masked(logits, mask), -1)
    out = torch.zeros((probs.shape[0], len(DEMO_KIND_GROUPS)),
                      dtype=probs.dtype, device=probs.device)
    index = DEMO_KIND_OF.to(probs.device).unsqueeze(0).expand_as(probs)
    return out.scatter_add(1, index, probs)


def demo_kind_cross_entropy(net, obs, mask, action):
    """The grouped masked policy's cross-entropy to Era's kind, averaged over
    rows: -log(sum of the legal actions' probabilities in Era's kind)."""
    logits, _ = net(obs)
    probs = demo_kind_probs(logits, mask)
    target = DEMO_KIND_OF.to(probs.device)[action.to(probs.device)]
    picked = probs.gather(1, target[:, None]).squeeze(1)
    return -picked.clamp_min(1e-12).log().mean()


def add_demo_kind_loss(loss, net, obs, mask, action, weight):
    """Add the kind-level demonstration term to a PPO loss. Weight 0 returns
    `loss` itself, so today's loss is unchanged exactly."""
    if weight == 0.0:
        return loss
    return loss + weight * demo_kind_cross_entropy(net, obs, mask, action)


@torch.no_grad()
def demo_agreement(net, demo, dev, batch=DEMO_EVAL_BATCH, with_kind=True):
    """Masked top-1 agreement with Era on the held-out rows, on buys, on the
    grouped kinds and on the fixed train sample (same rows at every eval)."""
    def count(rows):
        hits = total = buy_hits = buy_total = kind_hits = 0
        for i in range(0, len(rows), batch):
            part = rows[i:i + batch]
            mask = demo.mask[part].to(dev)
            action = demo.action[part].to(dev)
            logits, _ = net(demo.obs[part].to(dev))
            pred = masked(logits, mask).argmax(1)
            if with_kind:
                kind_pred = demo_kind_probs(logits, mask).argmax(1)
                kind_hits += int((kind_pred == demo.kind[part].to(dev)).sum())
            hit = pred == action
            buy = demo.is_buy[part].to(dev)
            hits += int(hit.sum())
            total += len(part)
            buy_hits += int((hit & buy).sum())
            buy_total += int(buy.sum())
        return hits, total, buy_hits, buy_total, kind_hits

    hits, total, buy_hits, buy_total, kind_hits = count(demo.heldout)
    train_hits, train_total, _, _, train_kind_hits = count(demo.train_fixed)
    vals = {
        "demo_top1": hits / total,
        "demo_top1_buy": buy_hits / buy_total if buy_total else float("nan"),
        "demo_top1_train": train_hits / train_total,
    }
    if with_kind:
        vals.update({
            "demo_kind_top1": kind_hits / total,
            "demo_kind_top1_train": train_kind_hits / train_total,
        })
    return vals


def print_base_demo_kind(demo):
    """BASE's held-out kind agreement once at startup. A missing or unreadable
    checkpoint is a note, not a reason to stop the run."""
    if not DEMO_BASE_CKPT.is_file():
        print(f"base {DEMO_BASE_CKPT}: not found, no reference kind agreement")
        return
    try:
        net = load_model(str(DEMO_BASE_CKPT), "cpu").eval()
        vals = demo_agreement(net, demo, "cpu")
    except (Exception, SystemExit) as e:
        print(f"base {DEMO_BASE_CKPT}: cannot read ({e}), no reference kind agreement")
        return
    print(f"base {DEMO_BASE_CKPT.parent.name}: kind top1 "
          f"{vals['demo_kind_top1']:.1%} heldout, "
          f"{vals['demo_kind_top1_train']:.1%} train")


def buy_prior_cross_entropy(logits, mask, prior_logits):
    """The prior's softmax over the legal buy slots as target, the network's
    log-softmax over the same slots as prediction, averaged over the rows with
    at least two legal buys (one legal slot is not a choice)."""
    legal = mask[:, BUY_ACTIONS]
    valid = legal.sum(1) >= 2
    if not valid.any():
        return logits.sum() * 0.0
    target_logits = prior_logits[valid].masked_fill(~legal[valid], -1e9)
    target = torch.softmax(target_logits, 1)
    buy_logits = logits[valid][:, BUY_ACTIONS].masked_fill(~legal[valid], -1e9)
    logpred = torch.log_softmax(buy_logits, 1)
    return -(target * logpred).sum(1).mean()


def add_buy_prior_loss(loss, logits, mask, prior_logits, weight):
    """Add the buy-prior term to a PPO loss. Weight 0 returns `loss` itself,
    so today's loss is unchanged exactly."""
    if weight == 0.0:
        return loss
    return loss + weight * buy_prior_cross_entropy(logits, mask, prior_logits)


def buy_prior_top1(logits, mask, prior_logits):
    """(hits, rows): rows with at least two legal buys where the network's
    top legal buy slot is the buy model's top legal buy slot."""
    legal = mask[:, BUY_ACTIONS]
    valid = legal.sum(1) >= 2
    if not valid.any():
        return 0, 0
    net_buy = logits[valid][:, BUY_ACTIONS].masked_fill(~legal[valid], -1e9)
    prior_buy = prior_logits[valid].masked_fill(~legal[valid], -1e9)
    return int((net_buy.argmax(1) == prior_buy.argmax(1)).sum()), int(valid.sum())


@torch.no_grad()
def buy_prior_agreement(logits, obs, mask, prior, ck, pidx, rows=None):
    """The buy-slot top-1 agreement for the selected rows (all rows when
    `rows` is None), with at least two legal buy slots. `logits` is the
    network's full action logits for `obs`; the prior runs on the CPU and
    `pidx` maps engine card ids to its vocabulary."""
    legal = mask[:, BUY_ACTIONS]
    valid = legal.sum(1) >= 2
    if rows is not None:
        valid = valid & rows
    if not valid.any():
        return 0, 0
    sub = valid.nonzero(as_tuple=False).squeeze(1)
    prior_logits = bp.prior_logits_for(
        obs[sub].detach().float().cpu().numpy(), pidx, prior, ck, "cpu",
        SHOP_COLS, BOARD_COLS, TURN,
        bgpy.SLOT_PRESENT, bgpy.SLOT_CARD_ID).to(obs.device)
    return buy_prior_top1(logits[sub], mask[sub], prior_logits)


def print_base_buy_prior(prior_data, dev, games=BASE_BUY_PRIOR_GAMES):
    """BASE's buy-slot agreement once at startup, on the standard-bot eval.
    A missing or unreadable checkpoint is a note, not a reason to stop."""
    if not DEMO_BASE_CKPT.is_file():
        print(f"base {DEMO_BASE_CKPT}: not found, no reference buy agreement")
        return
    try:
        net = load_model(str(DEMO_BASE_CKPT), dev).eval()
        _, _, agree = evaluate(net, min(games, 1024),
                               [1, 0, 7, 0, 0, 0], dev, 10_000,
                               buy_prior=prior_data)
    except (Exception, SystemExit) as e:
        print(f"base {DEMO_BASE_CKPT}: cannot read ({e}), no reference buy agreement")
        return
    hits, total = agree
    if not total:
        print(f"base {DEMO_BASE_CKPT.parent.name}: no buy decisions")
        return
    print(f"base {DEMO_BASE_CKPT.parent.name}: buy prior top1 {hits / total:.1%} "
          f"({hits}/{total} decisions, {min(games, 1024)} standard-bot games)")


def entropy_term(dist, moves_as_one, move_idx=None):
    """The entropy bonus: ordinary entropy, or with moves_as_one the summed
    probability of the move actions treated as one atom (as oneturn.py does).
    Off returns the plain term exactly, with no extra arithmetic."""
    if not moves_as_one:
        return dist.entropy().mean()
    probs = dist.probs
    if move_idx is None:
        move_idx = MOVE_IDX
    idx = torch.as_tensor(move_idx, dtype=torch.long, device=probs.device)
    move_p = probs[:, idx].sum(1)
    nonmove = probs.clone()
    nonmove[:, idx] = 0
    return (-(nonmove * torch.log(nonmove.clamp_min(1e-12))).sum(1)
            - move_p * torch.log(move_p.clamp_min(1e-12))).mean()


class Bufs:
    """The numpy arrays bgpy fills in, one set per VecEnv."""

    def __init__(self, n):
        self.obs = np.zeros((n, bgpy.OBS_SIZE), np.float32)
        self.mask = np.zeros((n, bgpy.N_ACTIONS), bool)
        self.seat = np.zeros(n, np.int64)
        self.kind = np.zeros(n, np.int64)
        self.places = np.zeros((n, P), np.int8)
        self.tier_turns = np.zeros((n, P, bgpy.MAX_TIER), np.int8)
        self.tribes = np.zeros((n, P, bgpy.TRIBES), np.int8)
        self.reset = np.zeros(n, bool)
        self.kinds = np.zeros((n, P), np.int64)
        self.forced = np.full((n, P), -1, np.int64)
        self.forced1 = np.full(n, -1, np.int64)
        self.bonus = np.zeros((n, P, bgpy.N_ACTIONS), np.float32)
        self.flags = np.zeros((n, P, bgpy.OVERLAY_FLAGS), np.int32)
        self.comp = np.full((n, P), -1, np.int64)
        # --fight-score: each seat's last fight, uncapped, from the engine.
        self.fight_dealt = np.zeros((n, P), np.int32)
        self.fight_taken = np.zeros((n, P), np.int32)


# --force-turn2-level: on turn 2 a tier 1 seat that can level must level
# (the curve every beginner is taught). Set from the flag in main().
FORCE_TURN2 = False
# --old-boards: "keep" or "drop", what an opponent's last-seen board is worth
# in the observation (bgpy's old_boards). Set from the flag in main(), so the
# eval games see what the training games see.
OLD_BOARDS = "keep"
# --real-lobby-filter: a real seat draws only boards whose minions fit this
# game's lobby (bgpy's real_lobby_filter). Set from the flag in main(), so
# every eval game sees what the training games see.
REAL_LOBBY_FILTER = False
# --learner-heroes: hero names every learner seat is given, one per game with
# the game's RNG; empty is the normal random hero deal. Set from the flag in
# main(), so the training games and every eval use the same list.
LEARNER_HEROES = []
# --real-lag-mix: the weighted lags training games draw from, and the lags the
# mixed-lag eval reports (sorted). Empty means every game is lag 0.
REAL_LAG_MIX = []
LAG_MIX_LAGS = []
# --real-lag-from: the first turn the real seats' lag applies on. Before it
# they draw their own turn as at lag 0. 1 lags every turn, the old behavior.
REAL_LAG_FROM = 1
LEVEL = int(np.nonzero(KIND_OF.numpy() == KINDS.index("level"))[0][0])
DARK = int(np.nonzero(KIND_OF.numpy() == KINDS.index("dark_gift"))[0][0])
MOVE = KINDS.index("move")
# --entropy-moves-as-one: the move actions are all of this kind, from bgpy's
# action layout rather than a hand-written index range.
MOVE_IDX = np.nonzero(KIND_OF.numpy() == MOVE)[0]
# --force-dark-gift: on turns 3, 4 and 5 every Python seat presses Dark
# Discovery first thing when it can (once per turn, the tutorial line). Set
# from the flag in main().
FORCE_DARK = False
DARK_TURNS = (3, 4, 5)
# --overlay-comps, --overlay-bonus and the comp file's health penalty. Set in
# main(); the eval games need the same comp input and comp specs.
OVERLAY_COMPS = None
OVERLAY_BONUS = 8.0
COMP_HEALTH_PENALTY = 0.0
COMP_SPECS = []
COMP_COUNT = 0
# --scenario-comps/--scenario-share/--scenario-turns: the comp names a
# scenario game may start with, the turn range, and the keys the scenario eval
# writes. Set in main; evals use the turn range so a scenario eval matches the
# training starts.
SCENARIO_NAMES = []
SCENARIO_TURN_MIN = 7
SCENARIO_TURN_MAX = 11
SCENARIO_LABELS = []  # (label, names, specs) for each comp and "pooled"
# --era-starts/--era-share/--era-turns: Era's logged turn-start positions, the
# share of training games that start from one, and the turn range. Evals use
# the holdout split; training draws from the train split.
ERA_STARTS = None
ERA_TURN_MIN = 7
ERA_TURN_MAX = 11
# --era-start-lag: the fixed lag of every late-start game (None: the same
# draw as a normal game). Set in main; the holdout Era eval uses it too.
ERA_START_LAG = None
ERA_METRICS = ("place", "top4", "first", "body_kept")
# The overlay flag order is in crates/bgcore/src/overlay.rs.
F_SHELL_LEGAL = 0
F_SHELL_TAKEN = 1
F_ACTIVE = 4
F_COMP = 6
# The actions an active comp's aux loss points at: buys, a Tavern spell, a
# Discover/Choose pick, and playing a comp card. A forced full-board sell is
# left out (it is not the "buy/pick" the brief names).
AUX_KINDS = {"buy", "buy_spell", "play", "choose"}
AUX_ACTIONS = np.zeros(bgpy.N_ACTIONS, bool)
for _kind, _n in bgpy.ACTION_KINDS:
    if _kind in AUX_KINDS:
        _start = sum(n for k, n in bgpy.ACTION_KINDS[:KINDS.index(_kind)])
        AUX_ACTIONS[_start:_start + _n] = True


def wheels_scale(games, full, end):
    """The wheels' strength after `games` games: 1 before `full`, linear to 0
    at `end`, 0 after. With both zero the fade is off and the wheels stay at
    full strength (the old behaviour)."""
    if full <= 0 and end <= 0:
        return 1.0
    if games < full:
        return 1.0
    if end <= full or games >= end:
        return 0.0 if games >= end else 1.0
    return (end - games) / (end - full)


def comp_onehot(idx, n_comps, dev):
    """A one-hot comp input, all zero for no active comp (index -1)."""
    if not n_comps:
        return None
    idx = torch.as_tensor(np.asarray(idx), dtype=torch.long)
    out = torch.zeros((idx.numel(), n_comps), dtype=torch.float32)
    ok = idx >= 0
    if ok.any():
        out[torch.arange(idx.numel())[ok], idx[ok]] = 1.0
    return out.to(dev)


class BonusStore:
    """The overlay bonus each row acted under, packed as four bitmasks.

    The overlay's bonus is a sum of at most four things: B, B/2, the Health
    penalty, and -inf. Storing the whole float32 array would be 48 MB for a
    64 by 1024 rollout. Four bitmasks of N_ACTIONS (23 bytes each) plus the
    raw bonus values and the wheel scale rebuild the same float32 array, so
    the PPO update sees exactly the policy that acted."""

    def __init__(self, rows):
        self.rows = rows
        self.nbytes = (bgpy.N_ACTIONS + 7) // 8
        self.masks = np.zeros((rows, 4, self.nbytes), np.uint8)
        self.b = 0.0
        self.half = 0.0
        self.hp = 0.0
        self.scale = 0.0

    def set_scale(self, scale, b, hp):
        self.scale = float(scale)
        self.b = float(b)
        self.half = float(b) * 0.5
        self.hp = float(hp)

    def put(self, rows, raw):
        """Store unscaled overlay bonuses `raw` (rows by N_ACTIONS)."""
        if self.scale <= 0.0:
            return
        bits = np.zeros((len(rows), 4, bgpy.N_ACTIONS), bool)
        values = [np.float32(self.b), np.float32(self.half), np.float32(self.hp)]
        # The bonus can be a sum (the Health rule and a comp bonus can land on
        # the same action), so mark every combination that adds up to it.
        for combo in range(1, 8):
            val = np.float32(0.0)
            for i in range(3):
                if combo >> i & 1:
                    val += values[i]
            hit = (raw == val) & (raw != 0)
            for i in range(3):
                if combo >> i & 1:
                    bits[:, i] |= hit
        bits[:, 3] = np.isneginf(raw)
        self.masks[rows] = np.packbits(bits, axis=-1)

    def get(self, sel, dev):
        """The scaled float32 bonus for the selected flat rows."""
        n = int(sel.sum())
        if self.scale <= 0.0:
            return torch.zeros((n, bgpy.N_ACTIONS), dtype=torch.float32, device=dev)
        bits = np.unpackbits(self.masks[sel], axis=-1)[..., :bgpy.N_ACTIONS]
        values = [np.float32(self.b), np.float32(self.half), np.float32(self.hp)]
        raw = np.zeros((n, bgpy.N_ACTIONS), np.float32)
        for i in range(3):
            raw += bits[:, i, :].astype(np.float32) * values[i]
        raw[bits[:, 3, :] > 0] = -np.inf
        raw *= np.float32(self.scale)
        return torch.from_numpy(raw).to(dev)


def force_mask(obs, mask):
    """The legal-action mask with forced moves: with --force-turn2-level, a
    seat on turn 2 at tier 1 must level; with --force-dark-gift, the Dark
    Gift press. The forced mask is the one stored for the update, so a forced
    action has log-probability 0 and no gradient."""
    rows = torch.zeros(obs.shape[0], dtype=torch.bool, device=obs.device)
    if FORCE_TURN2:
        rows |= (obs[:, bgpy.SCALAR_TURN] == 2) & (obs[:, bgpy.SCALAR_TIER] == 1)
    rows &= mask[:, LEVEL]
    dark = torch.zeros_like(rows)
    if FORCE_DARK:
        turn = obs[:, bgpy.SCALAR_TURN]
        on = torch.zeros_like(rows)
        for t in DARK_TURNS:
            on |= turn == t
        dark = on & mask[:, DARK]
    if not rows.any() and not dark.any():
        return mask
    mask = mask.clone()
    # Dark Discovery comes first: a seat forced to do both presses now and
    # levels on its next action.
    rows &= ~dark
    mask[rows] = False
    mask[rows, LEVEL] = True
    mask[dark] = False
    mask[dark, DARK] = True
    return mask


def act(net, obs, mask, greedy=False, bonus=None, forced=None, comp=None,
        return_logits=False):
    logits, value = net(obs, comp=comp)
    own_logits = logits
    if bonus is not None:
        logits = logits + bonus
    ml = masked(logits, mask)
    dist = torch.distributions.Categorical(logits=ml)
    a = ml.argmax(1) if greedy else dist.sample()
    if forced is not None:
        a = torch.where(forced >= 0, forced, a)
    if return_logits:
        return a, dist.log_prob(a), value, dist.entropy(), own_logits
    return a, dist.log_prob(a), value, dist.entropy()


@torch.no_grad()
def names_at_rev(rev):
    """Card, hero and trinket names by id as their TOML files stood at a git
    revision (no heroes before heroes.toml existed, no trinkets before
    trinkets.toml)."""
    cards = toml_names_at_rev(rev, "cards")
    try:
        heroes = toml_names_at_rev(rev, "heroes")
    except SystemExit:
        heroes = []
    try:
        trinkets = toml_names_at_rev(rev, "trinkets")
    except SystemExit:
        trinkets = []
    return cards, heroes, trinkets


def evaluate(net, n_games, lineup, dev, seed, real_boards=None, scenario_names=None,
             era_starts=None, era_split="train", real_lag_mix=None, era_start_lag=None,
             buy_prior=None):
    """_evaluate, retried on a new seed when a scripted bot hits the engine's
    illegal-action panic (a known engine bug, 2026-11-02), so one bad test
    game doesn't kill the training run. With `buy_prior` it also returns
    (hits, rows) for the buy-slot agreement; without it, the old two values."""
    for attempt in range(4):
        try:
            return _evaluate(net, n_games, lineup, dev, seed + attempt * 7919, real_boards,
                             scenario_names, era_starts, era_split, real_lag_mix,
                             era_start_lag, buy_prior)
        except BaseException as e:  # pyo3 PanicException is a BaseException
            if type(e).__name__ != "PanicException" or attempt == 3:
                raise
            print(f"eval panic ({e}); retrying with a new seed", flush=True)


def _evaluate(net, n_games, lineup, dev, seed, real_boards=None, scenario_names=None,
              era_starts=None, era_split="train", real_lag_mix=None, era_start_lag=None,
              buy_prior=None):
    """Play n_games with `lineup` and return (avg_place, learner_records), plus
    (hits, rows) as a third value when `buy_prior` is given.

    With --overlay-comps the comp input is on and the wheels are off: the
    overlay is only probed for its flags (no forced move and no bonus is
    applied). Each record carries the game's first active comp, final board,
    the turns the Shell was legal and taken, the game's drawn lag, and, with
    `scenario_names`, the scenario starting body, for the comp eval line.
    `buy_prior` is (model, checkpoint, card index); agree is its (hits, rows)
    on learner decisions with at least two legal buy slots, or None."""
    n = min(n_games, 1024)
    scenario = dict(scenario_comps=OVERLAY_COMPS, scenario_names=list(scenario_names),
                    scenario_share=1.0, scenario_turn_min=SCENARIO_TURN_MIN,
                    scenario_turn_max=SCENARIO_TURN_MAX) if scenario_names is not None else {}
    era_mode = era_starts is not None
    era = dict(era_starts=era_starts, era_share=1.0,
               era_turn_min=ERA_TURN_MIN, era_turn_max=ERA_TURN_MAX,
               era_split=era_split, era_start_lag=era_start_lag) if era_mode else {}
    env = bgpy.VecEnv(n, seed, lineup, real_boards=real_boards, old_boards=OLD_BOARDS,
                      real_lobby_filter=REAL_LOBBY_FILTER, real_lag_mix=real_lag_mix,
                      real_lag_from=REAL_LAG_FROM,
                      lag_in_obs=LAG_IN_OBS, learner_heroes=LEARNER_HEROES or None,
                      **scenario, **era)
    if OVERLAY_COMPS:
        env.set_overlay(OVERLAY_COMPS, 0.0)
    b = Bufs(n)
    env.kinds(b.kinds)
    records, done = [], 0
    place_seen = np.zeros((n, P), np.int8)
    tier_seen = np.zeros((n, P, bgpy.MAX_TIER), np.int8)
    tribe_seen = np.zeros((n, P, bgpy.TRIBES), np.int8)
    comp_turn = np.zeros((n, P), np.int16)
    start_body = np.full((n, P, bgpy.BOARD), -1, np.int16)
    era_turn = np.zeros((n, P), np.int16)
    if scenario_names is not None:
        start_body[:] = np.asarray(env.scenario_bodies(), dtype=np.int16)
    if era_mode:
        era_seats = np.asarray(env.era_seats(), dtype=np.int64)
        start_body[:] = np.asarray(env.era_bodies(), dtype=np.int16)
        era_turns = np.asarray(env.era_turns(), dtype=np.int64)
        for e in range(n):
            s = int(era_seats[e])
            if s >= 0:
                era_turn[e, s] = era_turns[e]
    shell_legal = [set() for _ in range(n * P)]
    shell_taken = [set() for _ in range(n * P)]
    ar = np.arange(n)
    lags = env.real_lags()
    learner_rows = torch.from_numpy(b.kind == LEARNER).to(dev)
    buy_hits = buy_rows = 0
    while done < n_games:
        env.observe(b.obs, b.mask, b.seat, b.kind)
        o = torch.from_numpy(b.obs).to(dev)
        m = force_mask(o, torch.from_numpy(b.mask).to(dev))
        comp_idx = np.full(n, -1, np.int64)
        if OVERLAY_COMPS:
            env.overlay_probe(b.forced1, b.flags)
            comp_idx = b.flags[ar, b.seat, F_COMP] - 1
        comp = comp_onehot(comp_idx, COMP_COUNT, dev)
        if buy_prior is None:
            a, _, _, _ = act(net, o, m, comp=comp)
        else:
            a, _, _, _, net_logits = act(net, o, m, comp=comp,
                                         return_logits=True)
            hits, rows = buy_prior_agreement(
                net_logits, o, m, *buy_prior, rows=learner_rows)
            buy_hits += hits
            buy_rows += rows
        acts = a.cpu().numpy()
        turns = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
        if OVERLAY_COMPS or era_mode:
            env.remember_boards()
        if OVERLAY_COMPS:
            for e in np.nonzero(b.kind == LEARNER)[0]:
                s = int(b.seat[e])
                key = e * P + s
                if comp_idx[e] >= 0 and comp_turn[e, s] == 0:
                    comp_turn[e, s] = turns[e]
                if b.flags[e, s, F_SHELL_LEGAL]:
                    shell_legal[key].add(int(turns[e]))
                    if b.forced1[e] == acts[e]:
                        shell_taken[key].add(int(turns[e]))
        kinds = b.kinds.copy()
        env.step(acts, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kinds[e, s] == LEARNER:
                place_seen[e, s] = b.places[e, s]
                tier_seen[e, s] = b.tier_turns[e, s]
                tribe_seen[e, s] = b.tribes[e, s]
        done += int(b.reset.sum())
        if b.reset.any():
            finals = env.final_boards() if OVERLAY_COMPS or era_mode else None
            fresh_body = np.asarray(env.scenario_bodies(), dtype=np.int16) \
                if scenario_names is not None else None
            fresh_era = np.asarray(env.era_bodies(), dtype=np.int16) if era_mode else None
            for e in np.nonzero(b.reset)[0]:
                for s in np.nonzero(kinds[e] == LEARNER)[0]:
                    key = e * P + s
                    records.append((
                        int(place_seen[e, s]),
                        tier_seen[e, s].copy(),
                        tribe_seen[e, s].copy(),
                        {
                            "comp_turn": int(comp_turn[e, s]),
                            "era_turn": int(era_turn[e, s]),
                            "real_lag": int(lags[e]),
                            "final": ([int(x) for x in finals[e][s] if x >= 0]
                                      if finals is not None else []),
                            "body_start": [int(x) for x in start_body[e, s] if x >= 0],
                            "shell_legal": shell_legal[key],
                            "shell_taken": shell_taken[key],
                        },
                    ))
                    place_seen[e, s] = 0
                    tier_seen[e, s] = 0
                    tribe_seen[e, s] = 0
                    comp_turn[e, s] = 0
                    shell_legal[key] = set()
                    shell_taken[key] = set()
                if fresh_body is not None:
                    start_body[e] = fresh_body[e]
                if fresh_era is not None:
                    start_body[e] = fresh_era[e]
            env.kinds(b.kinds)
            lags = env.real_lags()
    avg_place = float(np.mean([r[0] for r in records]))
    if buy_prior is None:
        return avg_place, records
    return avg_place, records, (buy_hits, buy_rows)


@torch.no_grad()
def evaluate_vs(net, other, n_games, dev, seed):
    """4 learner seats against 4 seats of `other`. Learners' average place."""
    n = min(n_games, 1024)
    env = bgpy.VecEnv(n, seed, [4, 4, 0, 0, 0, 0], old_boards=OLD_BOARDS,
                      lag_in_obs=LAG_IN_OBS, learner_heroes=LEARNER_HEROES or None)
    if OVERLAY_COMPS:
        env.set_overlay(OVERLAY_COMPS, 0.0)
    b = Bufs(n)
    env.kinds(b.kinds)
    places, done = [], 0
    ar = np.arange(n)
    while done < n_games:
        env.observe(b.obs, b.mask, b.seat, b.kind)
        o, m = torch.from_numpy(b.obs).to(dev), torch.from_numpy(b.mask).to(dev)
        m = force_mask(o, m)
        comp_idx = np.full(n, -1, np.int64)
        if OVERLAY_COMPS:
            env.overlay_probe(b.forced1, b.flags)
            comp_idx = b.flags[ar, b.seat, F_COMP] - 1
        comp = comp_onehot(comp_idx, COMP_COUNT, dev)
        a, _, _, _ = act(net, o, m, comp=comp)
        sn = torch.from_numpy(b.kind == SNAPSHOT).to(dev)
        if sn.any():
            a[sn] = act(other, o[sn], m[sn],
                        comp=comp[sn] if comp is not None else None)[0]
        kinds = b.kinds.copy()
        env.step(a.cpu().numpy(), b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kinds[e, s] == LEARNER:
                places.append(b.places[e, s])
        done += int(b.reset.sum())
        if b.reset.any():
            env.kinds(b.kinds)
    return float(np.mean(places))


def summarize_records(records):
    """Compute curve and tribe reports from finished learner seat records."""
    places = np.array([r[0] for r in records], dtype=np.float64)
    tiers = np.array([r[1] for r in records], dtype=np.int32)
    tribes = np.array([r[2] for r in records], dtype=np.int32)

    t3, t4, t5 = tiers[:, 2], tiers[:, 3], tiers[:, 4]
    turn_t3 = float(t3[t3 > 0].mean()) if (t3 > 0).any() else float("nan")
    turn_t4 = float(t4[t4 > 0].mean()) if (t4 > 0).any() else float("nan")
    turn_t5 = float(t5[t5 > 0].mean()) if (t5 > 0).any() else float("nan")

    econ_rush = (t3 > 0) & (t3 <= 3)
    rush = ~econ_rush & (t3 > 0) & (t3 <= 4) & (t4 > 0) & (t4 <= 5)
    standard = ~econ_rush & ~rush & (t3 > 0) & (t3 <= 5) & (t4 > 0) & (t4 <= 7)
    slow = ~(econ_rush | rush | standard)

    # A board's tribe is the most common of the ten when at least 4 minions
    # have it (a dual or all-type minion counts for each); otherwise mixed.
    counts = tribes[:, list(TRIBE_COLS.values())]
    main = counts.argmax(1)
    has_main = counts.max(1) >= 4
    groups = {t: has_main & (main == k) for k, t in enumerate(TRIBE_COLS)}
    groups["mixed"] = ~has_main
    tribe_rep = {}
    for t, sel in groups.items():
        tribe_rep[f"tribe_{t}"] = float(sel.mean())
        tribe_rep[f"place_{t}"] = float(places[sel].mean()) if sel.any() else float("nan")

    return {
        "turn_t3": turn_t3,
        "turn_t4": turn_t4,
        "turn_t5": turn_t5,
        "curve_econ_rush": float(econ_rush.mean()),
        "curve_rush": float(rush.mean()),
        "curve_standard": float(standard.mean()),
        "curve_slow": float(slow.mean()),
        **tribe_rep,
    }


def summarize_comps(records, specs):
    """The wheels-off comp line from finished learner records: games with a
    comp active, of those the share ending as the comp, the Shell taken per
    legal turn, and the first-place share."""
    places = np.array([r[0] for r in records], dtype=np.float64)
    active = [r for r in records if r[3]["comp_turn"] > 0]
    ended = sum(
        1 for r in active
        if any(comp_specs.ends_as_comp(r[3]["final"], spec) for spec in specs)
    )
    legal = sum(len(r[3]["shell_legal"]) for r in records)
    taken = sum(len(r[3]["shell_taken"]) for r in records)
    return {
        "comp_active_share": float(len(active) / len(records)) if records else 0.0,
        "comp_end_share": float(ended / len(active)) if active else 0.0,
        "shell_rate": float(taken / legal) if legal else 0.0,
        "first_share": float((places == 1).mean()) if records else 0.0,
    }


def scenario_slug(name):
    """A metrics.csv-safe label for a comp's scenario columns."""
    out = "".join(c.lower() if c.isalnum() else "_" for c in name)
    while "__" in out:
        out = out.replace("__", "_")
    return out.strip("_") or "comp"


def summarize_scenario(records, specs):
    """The scenario eval's numbers: average place, top-4 and first place, the
    share ending as the comp (the comp eval line's test), and the share of the
    starting body's minions no longer on the final board.

    The body is a multiset of card ids: a starting copy counts as kept when
    the final board has a copy of that card left. A body that triple-merged
    therefore counts the two merged copies as gone, which is the closest the
    final card ids can say."""
    places = np.array([r[0] for r in records], dtype=np.float64)
    ended = sum(
        1 for r in records
        if any(comp_specs.ends_as_comp(r[3]["final"], spec) for spec in specs)
    )
    sold, sold_games = 0, 0
    for r in records:
        start = r[3]["body_start"]
        if not start:
            continue
        left = list(r[3]["final"])
        kept = 0
        for card in start:
            if card in left:
                left.remove(card)
                kept += 1
        sold += len(start) - kept
        sold_games += len(start)
    return {
        "place": float(places.mean()) if records else float("nan"),
        "top4": float((places <= 4).mean()) if records else 0.0,
        "first": float((places == 1).mean()) if records else 0.0,
        "end": float(ended / len(records)) if records else 0.0,
        "body_sold": float(sold / sold_games) if sold_games else float("nan"),
    }


def summarize_era(records):
    """The Era eval: average place, top 4, firsts, and the share of the
    starting board's card copies still on the final board. A copy counts as
    kept when the final board has a card of that id left (the same multiset
    rule as the scenario body)."""
    places = np.array([r[0] for r in records], dtype=np.float64)
    kept, total = 0, 0
    for r in records:
        start = r[3]["body_start"]
        if not start:
            continue
        left = list(r[3]["final"])
        for card in start:
            if card in left:
                left.remove(card)
                kept += 1
        total += len(start)
    return {
        "place": float(places.mean()) if records else float("nan"),
        "top4": float((places <= 4).mean()) if records else 0.0,
        "first": float((places == 1).mean()) if records else 0.0,
        "body_kept": kept / total if total else float("nan"),
    }


def main():
    global FORCE_TURN2, FORCE_DARK, REAL_LOBBY_FILTER
    global REAL_LAG_MIX, LAG_MIX_LAGS, REAL_LAG_FROM
    global OVERLAY_COMPS, OVERLAY_BONUS, COMP_HEALTH_PENALTY, COMP_SPECS, COMP_COUNT
    global SCENARIO_NAMES, SCENARIO_TURN_MIN, SCENARIO_TURN_MAX, SCENARIO_LABELS
    global ERA_STARTS, ERA_TURN_MIN, ERA_TURN_MAX, ERA_START_LAG
    global LEARNER_HEROES
    cfg = args()
    LEARNER_HEROES = [name.strip() for name in (cfg.learner_heroes or "").split(",")
                      if name.strip()]
    cfg.learner_heroes = LEARNER_HEROES
    if cfg.first_bonus or cfg.second_bonus:
        print(f"placement reward with first bonus {cfg.first_bonus:g} and "
              f"second bonus {cfg.second_bonus:g}:")
        print("  " + "  ".join(
            f"{p}: {reward(p, cfg.first_bonus, cfg.second_bonus):.2f}"
            for p in range(1, P + 1)))
    want_text = cfg.card_text
    want_power = cfg.power_text
    FORCE_TURN2 = cfg.force_turn2_level
    FORCE_DARK = cfg.force_dark_gift
    REAL_LOBBY_FILTER = cfg.real_lobby_filter
    if cfg.real_lag_mix:
        mix = []
        try:
            for part in cfg.real_lag_mix.split(","):
                lag, weight = part.split(":")
                mix.append((int(lag), int(weight)))
        except ValueError:
            raise SystemExit(f"--real-lag-mix {cfg.real_lag_mix}: needs LAG:WEIGHT pairs, "
                             "e.g. 0:1,1:1,2:1")
        if any(lag < 0 or weight < 0 for lag, weight in mix) or sum(w for _, w in mix) <= 0:
            raise SystemExit(f"--real-lag-mix {cfg.real_lag_mix}: lags and weights must be "
                             "non-negative and at least one weight positive")
        REAL_LAG_MIX = mix
        LAG_MIX_LAGS = sorted({lag for lag, weight in mix if weight > 0})
    if cfg.real_lag_from < 0:
        raise SystemExit(f"--real-lag-from {cfg.real_lag_from}: the first turn cannot be negative")
    REAL_LAG_FROM = cfg.real_lag_from
    OVERLAY_COMPS = cfg.overlay_comps
    OVERLAY_BONUS = cfg.overlay_bonus
    raw_specs = []
    if cfg.overlay_comps:
        COMP_COUNT = comp_specs.count(cfg.overlay_comps)
        COMP_HEALTH_PENALTY = comp_specs.health_penalty(cfg.overlay_comps)
        raw_specs = comp_specs.load_comps(cfg.overlay_comps)
    if cfg.scenario_comps:
        if not cfg.overlay_comps:
            raise SystemExit("--scenario-comps needs --overlay-comps: the names come from that file")
        SCENARIO_NAMES = [n.strip() for n in cfg.scenario_comps.split(";") if n.strip()]
        known = {s["name"] for s in raw_specs}
        for name in SCENARIO_NAMES:
            if name not in known:
                raise SystemExit(f"--scenario-comps: no comp named {name!r} in {cfg.overlay_comps}")
        try:
            lo, hi = (int(x) for x in cfg.scenario_turns.split(","))
        except ValueError:
            raise SystemExit(f"--scenario-turns {cfg.scenario_turns}: needs MIN,MAX")
        if lo < 1 or hi < lo:
            raise SystemExit(f"--scenario-turns {cfg.scenario_turns}: need 1 <= MIN <= MAX")
        SCENARIO_TURN_MIN, SCENARIO_TURN_MAX = lo, hi
    if cfg.era_starts:
        try:
            lo, hi = (int(x) for x in cfg.era_turns.split(","))
        except ValueError:
            raise SystemExit(f"--era-turns {cfg.era_turns}: needs MIN,MAX")
        if lo < 1 or hi < lo:
            raise SystemExit(f"--era-turns {cfg.era_turns}: need 1 <= MIN <= MAX")
        ERA_STARTS = cfg.era_starts
        ERA_TURN_MIN, ERA_TURN_MAX = lo, hi
        if cfg.era_share < 0 or cfg.era_share > 1:
            raise SystemExit(f"--era-share {cfg.era_share}: needs 0 to 1")
    elif cfg.era_share:
        raise SystemExit("--era-share needs --era-starts DIR")
    if cfg.era_start_lag is not None:
        if not cfg.era_starts:
            raise SystemExit("--era-start-lag needs --era-starts DIR")
        if cfg.era_start_lag < 0:
            raise SystemExit("--era-start-lag: the lag cannot be negative")
        ERA_START_LAG = cfg.era_start_lag
    # a training setting, not a model one: the checkpoint and its weights are
    # the same with the blocks recomputed or kept
    set_recompute(not cfg.no_recompute)
    torch.manual_seed(cfg.seed)
    random.seed(cfg.seed)
    dev = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    if cfg.vram_gb and dev.type == "cuda":
        total = torch.cuda.get_device_properties(0).total_memory / 2**30
        torch.cuda.set_per_process_memory_fraction(min(1.0, cfg.vram_gb / total), 0)
    ck = torch.load(cfg.resume, map_location=dev) if cfg.resume else None
    if ck is not None:
        if cfg.init_from:
            raise SystemExit("--resume and --init-from don't go together")
        saved_lag = (ck.get("cfg") or {}).get("lag_in_obs", False)
        if saved_lag != cfg.lag_in_obs:
            raise SystemExit(
                f"--lag-in-obs {'on' if cfg.lag_in_obs else 'off'} but {cfg.resume} was "
                f"trained with it {'on' if saved_lag else 'off'}: the observation width differs")
        saved, opts = ck_model(ck)
        given = {"model": cfg.model, **{k: getattr(cfg, k) for k in model_opts(saved)
                                        if hasattr(cfg, k)
                                        and k not in ("card_text", "power_text")}}
        for k, v in given.items():
            if v and v != {"model": saved, **opts}[k]:
                print(f"--{k.replace('_', '-')} {v} ignored: {cfg.resume} holds "
                      f"{saved} {opts_text(opts)}")
        if want_text and not opts.get("card_text"):
            print(f"--card-text ignored: {cfg.resume} has no card text table")
        if want_power and not opts.get("power_text"):
            print(f"--power-text: {cfg.resume} has no power text; its rows and gates start fresh")
        cfg.model = saved
        if opts.get("n_comps", 0) != COMP_COUNT:
            raise SystemExit(f"--overlay-comps {cfg.overlay_comps}: the comp file has "
                             f"{COMP_COUNT} comps but {cfg.resume} was built for "
                             f"{opts.get('n_comps', 0)}")
    else:
        if cfg.init_from and not cfg.model:
            cfg.model = ck_model(torch.load(cfg.init_from, map_location="cpu"))[0]
        cfg.model = cfg.model or "mlp"
        opts = model_opts(cfg.model, opp_boards=cfg.opp_boards, d=cfg.d, layers=cfg.layers,
                          heads=cfg.heads, ff=cfg.ff)
        opts["n_comps"] = COMP_COUNT
    for k, v in opts.items():
        setattr(cfg, k, v)
    if cfg.init_from:
        src_kind = ck_model(torch.load(cfg.init_from, map_location="cpu"))[0]
        if src_kind != cfg.model:
            raise SystemExit(f"--init-from {cfg.init_from} holds {src_kind}, not "
                             f"{cfg.model}: nothing to carry over")
    out = Path("runs") / cfg.name
    out.mkdir(parents=True, exist_ok=True)
    lineup = [int(x) for x in cfg.lineup.split(",")]
    if len(lineup) != bgpy.SEAT_KINDS:
        raise SystemExit(f"--lineup needs {bgpy.SEAT_KINDS} numbers "
                         f"(learner,snapshot,standard,rush,random,real), got {cfg.lineup}")
    # Real seats need the boards; --real-boards alone still adds the vs7 real
    # eval, so a run can watch that number without training against them.
    real = None
    if lineup[bgpy.REAL] > 0 or cfg.real_boards or cfg.era_starts:
        folder = cfg.real_boards or real_boards.DEFAULT_DIR
        real, unknown = real_boards.load(folder)
        print(f"real boards from {folder}:")
        real_boards.print_summary(real, unknown)
        if not real:
            raise SystemExit(f"no real boards in {folder}")
    if REAL_LAG_MIX and (real is None or lineup[bgpy.REAL] == 0):
        raise SystemExit("--real-lag-mix needs real seats and real boards")

    global OLD_BOARDS
    saved_boards = (ck.get("cfg") or {}).get("old_boards") if ck is not None else None
    OLD_BOARDS = cfg.old_boards or saved_boards or "keep"
    if saved_boards and cfg.old_boards and cfg.old_boards != saved_boards:
        print(f"--old-boards {cfg.old_boards}: the run was training with {saved_boards}")
    cfg.old_boards = OLD_BOARDS
    print(f"opponents' old boards: {OLD_BOARDS}")
    N, T = cfg.envs, cfg.steps
    if SCENARIO_NAMES and real is None:
        raise SystemExit("--scenario-comps needs real boards: the starting body is a real board")
    scenario_kw = (
        dict(scenario_comps=cfg.overlay_comps, scenario_names=SCENARIO_NAMES,
             scenario_share=cfg.scenario_share, scenario_turn_min=SCENARIO_TURN_MIN,
             scenario_turn_max=SCENARIO_TURN_MAX)
        if SCENARIO_NAMES else {}
    )
    era_kw = (
        dict(era_starts=cfg.era_starts, era_share=cfg.era_share,
             era_turn_min=ERA_TURN_MIN, era_turn_max=ERA_TURN_MAX,
             era_split="train", era_start_lag=ERA_START_LAG)
        if cfg.era_starts else {}
    )
    env = bgpy.VecEnv(N, cfg.seed, lineup, real_boards=real, old_boards=OLD_BOARDS,
                      real_lobby_filter=REAL_LOBBY_FILTER, real_lag_mix=REAL_LAG_MIX,
                      real_lag_from=REAL_LAG_FROM,
                      lag_in_obs=LAG_IN_OBS, learner_heroes=LEARNER_HEROES or None,
                      **scenario_kw, **era_kw)
    if cfg.overlay_comps:
        env.set_overlay(cfg.overlay_comps, cfg.overlay_bonus)
        print(f"overlay {cfg.overlay_comps} B {cfg.overlay_bonus:g}")
    names = env.card_names()
    hero_names = env.hero_names()
    trinket_names = env.trinket_names()
    power_fresh = False
    if ck is None and (want_text or want_power):
        if want_text and want_power:
            opts["card_text"], opts["power_text"] = card_text.build_text(
                names, hero_names, trinket_names)
            if cfg.card_form:
                opts["card_text"].update(card_text.build_forms(names))
            cfg.card_text = opts["card_text"]
            cfg.power_text = opts["power_text"]
            power_fresh = True
        elif want_text and not opts.get("card_text"):
            opts["card_text"] = card_text.build_table(names)
            if cfg.card_form:
                opts["card_text"].update(card_text.build_forms(names))
            cfg.card_text = opts["card_text"]
    if want_power and not opts.get("power_text"):
        opts["power_text"] = card_text.build_power_table(
            hero_names, trinket_names,
            vocab=(opts.get("card_text") or {}).get("vocab"))
        if opts.get("card_text"):
            opts["card_text"] = card_text.remap_table(
                opts["card_text"], opts["power_text"]["vocab"])
            cfg.card_text = opts["card_text"]
        cfg.power_text = opts["power_text"]
        power_fresh = True
    if opts.get("power_text"):
        table = opts["power_text"]
        missing = table.get("missing") or {}
        print(f"power text: {len(table['powers']) - 1} hero powers, "
              f"{len(table['trinkets']) - 1} trinkets, shared vocabulary "
              f"{len(table['vocab'])} words, "
              f"{len(missing.get('powers', []))} + {len(missing.get('trinkets', []))} "
              f"without text"
              + (" (fresh; gates 0)" if power_fresh else ""))
    if opts.get("card_text"):
        table = opts["card_text"]
        print(f"card text: {len(table['vocab'])} words, {len(table['tokens'])} cards, "
              f"{len(table['tokens'][0])} tokens, {len(table.get('missing', []))} without text")
        if table.get("forms"):
            print(f"card forms: {len(table['form_vocab'])} values, "
                  f"{len(table.get('form_missing', []))} cards without a form")
    COMP_SPECS = comp_specs.ids(raw_specs, names) if raw_specs else []
    if SCENARIO_NAMES:
        by_name = {s["name"]: s for s in COMP_SPECS}
        SCENARIO_LABELS = [(name, [name], [by_name[name]]) for name in SCENARIO_NAMES]
        SCENARIO_LABELS.append(
            ("pooled", list(SCENARIO_NAMES), [by_name[name] for name in SCENARIO_NAMES]))
        print(f"scenario starts: {cfg.scenario_share:.0%} of games, turns "
              f"{SCENARIO_TURN_MIN}..{SCENARIO_TURN_MAX}, comps {', '.join(SCENARIO_NAMES)}")
    if cfg.era_starts:
        era_lag_txt = (f"lag {ERA_START_LAG}" if ERA_START_LAG is not None
                       else "lag from the mix")
        print(f"Era starts: {cfg.era_share:.0%} of games, turns "
              f"{ERA_TURN_MIN}..{ERA_TURN_MAX}, train split, {era_lag_txt}, "
              f"{cfg.era_starts}")
    if REAL_LAG_MIX:
        print("real lag mix: " + " ".join(
            f"{lag}:{weight:g}" for lag, weight in REAL_LAG_MIX)
            + f"  from turn {REAL_LAG_FROM}"
            + (f"  lag in obs (OBS_SIZE {bgpy.OBS_SIZE})" if LAG_IN_OBS else ""))
    elif LAG_IN_OBS:
        print(f"lag in obs: on (OBS_SIZE {bgpy.OBS_SIZE})")
    print("learner heroes: "
          + (", ".join(LEARNER_HEROES) if LEARNER_HEROES else "random (the normal deal)"))
    if cfg.overlay_comps:
        print(f"overlay {cfg.overlay_comps}: {COMP_COUNT} comps, B {cfg.overlay_bonus:g}, "
              f"wheels full {cfg.wheels_full} end {cfg.wheels_end}, lambda {cfg.aux_weight:g}")
    if cfg.compile is None:
        cfg.compile = cfg.model == "attn"
    net = build(cfg.model, len(names), compile=cfg.compile, **opts).to(dev)
    if cfg.init_from:
        old_cards = old_heroes = old_trinkets = None
        if cfg.init_cards_rev:
            old_cards, old_heroes, old_trinkets = names_at_rev(cfg.init_cards_rev)
        loaded, fresh, skipped, src_opts, remapped = warm_start(
            net, cfg.model, cfg.init_from, cards=names, heroes=hero_names,
            trinkets=trinket_names, old_cards=old_cards, old_heroes=old_heroes,
            old_trinkets=old_trinkets)
        print(f"init from {cfg.init_from} ({cfg.model} {opts_text(src_opts)}): {len(loaded)} parameter "
              f"tensors loaded, {len(fresh)} fresh: {', '.join(fresh) or 'none'}; "
              f"not used: {', '.join(skipped) or 'none'}")
        for r in remapped:
            print(f"  carried: {r}")
    demo = None
    demo_gen = None
    if cfg.demo:
        if OLD_BOARDS != "keep":
            raise SystemExit("--demo: the dataset was built with old_boards='keep'; "
                             "this run uses old_boards='drop'")
        # The network reads bgpy's layout constants when it is built, so this
        # is the observation width it takes.
        demo = load_demo(cfg.demo, bgpy.OBS_SIZE)
        demo_gen = torch.Generator().manual_seed(cfg.seed)
        print(f"demo {cfg.demo}: {len(demo.train)} train, {len(demo.heldout)} heldout rows, "
              f"weight {cfg.demo_weight:g}"
              + (f", kind weight {cfg.demo_kind_weight:g}"
                 if cfg.demo_kind_weight else "")
              + (f", fading over the last {cfg.demo_fade:g} of the run"
                 if cfg.demo_fade else ""))
        if cfg.demo_kind_weight:
            print_base_demo_kind(demo)
    buy_prior_model = buy_prior_ck = buy_prior_pidx = buy_prior_data = None
    if cfg.buy_prior_weight > 0.0:
        prior_path = Path(cfg.buy_prior)
        if not prior_path.is_file():
            raise SystemExit(f"--buy-prior {prior_path}: does not exist")
        try:
            buy_prior_model, buy_prior_ck = bp.load_prior(prior_path, "cpu")
        except (Exception, SystemExit) as e:
            raise SystemExit(f"--buy-prior {prior_path}: cannot read: {e}")
        buy_prior_pidx = bp.make_pidx(names, buy_prior_ck, "cpu")
        buy_prior_data = (buy_prior_model, buy_prior_ck, buy_prior_pidx)
        print(f"buy prior {prior_path}: weight {cfg.buy_prior_weight:g}"
              + (f", fading over the last {cfg.buy_prior_fade:g} of the run"
                 if cfg.buy_prior_fade else ""))
        print_base_buy_prior(buy_prior_data, dev)
    opt = torch.optim.Adam(net.parameters(), lr=cfg.lr, eps=1e-5)
    pool = []  # snapshot state dicts, on the CPU
    update, best, t_total = 0, 9.0, 0.0
    # best.pt and --stop-flat follow vs7 real when there are real boards
    best_by = "real" if real is not None else "std+rush"
    if ck is not None:
        try:
            net.load_state_dict(ck["net"])
        except RuntimeError:
            # A checkpoint from before the 2026-11-06 opponent view has
            # narrower input layers, or one from before --power-text has the
            # new text buffers and gates missing or wider. Carry the old
            # columns/tables over (the network answers as it did until the
            # new parts learn), and start Adam and the snapshot pool fresh,
            # since their tensors and the pool's state dicts have the old
            # shapes.
            loaded, fresh, _, _, carried = warm_start(
                net, cfg.model, cfg.resume, cards=names, heroes=hero_names,
                trinkets=trinket_names)
            why = ("the new power text is fresh" if power_fresh
                   else "its observation is narrower than this build's")
            print(f"resumed {cfg.resume}: {why}, so "
                  f"{len(loaded)} tensors loaded, {len(fresh)} fresh: "
                  f"{', '.join(fresh) or 'none'}")
            for r in carried:
                print(f"  {r}")
            opt = torch.optim.Adam(net.parameters(), lr=cfg.lr, eps=1e-5)
            pool, update, best, t_total = [], ck["update"], ck["best"], ck.get("t_total", 0.0)
        else:
            opt.load_state_dict(ck["opt"])
            for g in opt.param_groups:
                g["lr"] = cfg.lr
            pool, update, best, t_total = ck["pool"], ck["update"], ck["best"], ck.get("t_total", 0.0)
        if ck.get("best_by", "std+rush") != best_by:
            best = 9.0  # best.pt was chosen by another eval: start over
        print(f"resumed {cfg.resume} at update {update}")
    gates = text_gates(net)
    if gates:
        print("power text gates: " + ", ".join(
            f"{name} {value:.6f}" for name, value in gates))
    if not pool:
        pool.append(copy.deepcopy({k: v.cpu() for k, v in net.state_dict().items()}))
    snap = build(cfg.model, len(names), compile=cfg.compile, **opts).to(dev)
    kind_of = KIND_OF.to(dev)
    cost_kinds = set(KINDS) if cfg.cost_kinds == "all" else set(cfg.cost_kinds.split(","))
    if cost_kinds - set(KINDS):
        raise SystemExit(f"--cost-kinds: unknown {sorted(cost_kinds - set(KINDS))}, kinds are {KINDS}")
    cost_of = torch.tensor([cfg.action_cost if k in cost_kinds else 0.0 for k in KINDS], device=dev)
    # the standard curve bots.rs levels on: the tier it wants by turn 1..12+
    curve = torch.tensor([1, 2, 2, 2, 3, 3, 4, 4, 4, 4, 5, 6], device=dev, dtype=torch.float32)
    # Era's deadlines, inclusive, from --deadline-curve (index = turn - 1, the
    # last entry holds for later turns)
    late_need = torch.tensor([float(x) for x in cfg.deadline_curve.split(",")],
                             device=dev, dtype=torch.float32)
    if cfg.fight_score:
        print(f"fight score {cfg.fight_score:g}: fights after turns "
              f"{cfg.fight_score_from}+, (uncapped dealt - taken) / "
              f"{FIGHT_SCORE_SCALE:g}"
              + (f", fading over the last {cfg.fight_score_fade:g} of the run"
                 if cfg.fight_score_fade else ""))
    if cfg.win_score:
        print(f"win score {cfg.win_score:g}: +W win, W/2 tie, 0 loss, fights after turns "
              f"{cfg.win_score_from}+"
              + (f", fading over the last {cfg.win_score_fade:g} of the run"
                 if cfg.win_score_fade else ""))
    print(f"{cfg.model} {opts_text(opts) or ''}: "
          f"{sum(p.numel() for p in net.parameters()) / 1e6:.2f}M "
          f"parameters on {dev}{', compiled' if cfg.compile and cfg.model == 'attn' else ''}")

    tb = SummaryWriter(out / "tb")
    csv_path = out / "metrics.csv"
    new_csv = not csv_path.exists()
    csv_file = open(csv_path, "a", newline="")
    log = csv.writer(csv_file)
    scen_metrics = ("place", "top4", "first", "end", "body_sold")
    eval_keys = list(EVAL_KEYS) + [
        f"scen_{scenario_slug(label)}_{metric}"
        for label, _, _ in SCENARIO_LABELS for metric in scen_metrics
    ] + ([f"era_{metric}" for metric in ERA_METRICS] if cfg.era_starts else [])
    if REAL_LAG_MIX:
        eval_keys += ["place_vs_real_mix"] + [
            f"real_lag_{lag}_{metric}"
            for lag in LAG_MIX_LAGS for metric in ("share", "place")
        ]
    if demo is not None:
        eval_keys += DEMO_KEYS
        if cfg.demo_kind_weight:
            eval_keys += DEMO_KIND_KEYS
    if buy_prior_model is not None:
        eval_keys += BUY_PRIOR_KEYS
    if new_csv:
        log.writerow([
            "update", "minutes", "games",
            *eval_keys,
            *TRAIN_KEYS,
            *[f"act_{k}" for k in KINDS],
            "top_buys",
            *EXTRA_KEYS,
        ])

    b = Bufs(N)
    env.kinds(b.kinds)
    # --obs-fp16: the rollout's observations in float16 (rounded once, when
    # they are stored) and widened again just before the network, which wants
    # float32; halves the buffer, which is the run's largest permanent tensor
    obs_dtype = torch.float16 if cfg.obs_fp16 else torch.float32
    obs = torch.zeros((T, N, bgpy.OBS_SIZE), device=dev, dtype=obs_dtype)
    masks = torch.zeros((T, N, bgpy.N_ACTIONS), dtype=torch.bool, device=dev)
    acts = torch.zeros((T, N), dtype=torch.long, device=dev)
    logps = torch.zeros((T, N), device=dev)
    vals = torch.zeros((T, N), device=dev)
    rews = torch.zeros((T, N), device=dev)
    dones = torch.zeros((T, N), device=dev)
    learn = torch.zeros((T, N), dtype=torch.bool, device=dev)
    seats = torch.zeros((T, N), dtype=torch.long, device=dev)
    # The comp input (index or -1), the overlay rows whose action was forced,
    # and the aux loss's target action, for each rollout row.
    comp_idx = np.full((T, N), -1, np.int64)
    forced_rows = np.zeros((T, N), bool)
    aux_target = np.full((T, N), -1, np.int64)
    comp_active = np.zeros((T, N), bool)
    bonus_store = BonusStore(T * N)
    # step of each (env, seat)'s latest learner sample in this rollout, -1 for none
    last = np.full((N, P), -1, np.int64)
    # The wheels fade on games, so a restart continues from the checkpoint's
    # count instead of putting them back to full strength.
    games = int(ck.get("games", 0)) if ck is not None else 0
    start = time.time()
    # The planned end is --minutes (go.sh passes the run's absolute final
    # minute); the fade covers the last fraction of this process's run.
    run_start_min = t_total
    last_keep = t_total
    history, flat_best, flat_since = [], 9.0, t_total
    stats = {}

    while True:
        snap.load_state_dict(random.choice(pool))
        last.fill(-1)
        rews.zero_()
        dones.zero_()
        t0 = time.time()
        # a shove off the low line, not a goal: it fades to 0 so placement
        # alone decides in the end (docs/briefs/training-strategy.md)
        run_min = t_total + (t0 - start) / 60
        low_pen = cfg.low_penalty * max(0.0, 1.0 - run_min / cfg.low_penalty_fade) if cfg.low_penalty_fade > 0 else cfg.low_penalty
        if run_min >= cfg.deadline_end:
            dl_scale = 0.0
        elif run_min <= cfg.deadline_full:
            dl_scale = 1.0
        else:
            dl_scale = (cfg.deadline_end - run_min) / (cfg.deadline_end - cfg.deadline_full)
        wheel = wheels_scale(games, cfg.wheels_full, cfg.wheels_end)
        if cfg.overlay_bonus == 0.0 and cfg.aux_weight == 0.0:
            # B = 0 and lambda = 0 is the old PPO: no forced move, no bonus,
            # no aux loss, and the comp input is the only thing left on.
            wheel = 0.0
        aux_w = cfg.aux_weight * wheel
        bonus_store.set_scale(wheel, cfg.overlay_bonus, COMP_HEALTH_PENALTY)
        # --fight-score-fade: the weight falls linearly to 0 at the run's
        # planned end, over its last fraction F (of this process's run).
        score_scale = fight_score_fade_scale(
            run_min, run_start_min, cfg.minutes, cfg.fight_score_fade)
        score_weight = cfg.fight_score * score_scale
        # --win-score-fade: the same fade, by the same planned end.
        win_scale = win_score_fade_scale(
            run_min, run_start_min, cfg.minutes, cfg.win_score_fade)
        win_weight = cfg.win_score * win_scale
        demo_weight = (cfg.demo_weight * demo_fade_scale(
            run_min, run_start_min, cfg.minutes, cfg.demo_fade)
            if demo is not None else 0.0)
        demo_kind_weight = (cfg.demo_kind_weight * demo_fade_scale(
            run_min, run_start_min, cfg.minutes, cfg.demo_fade)
            if demo is not None else 0.0)
        buy_prior_weight = (cfg.buy_prior_weight * demo_fade_scale(
            run_min, run_start_min, cfg.minutes, cfg.buy_prior_fade)
            if buy_prior_model is not None else 0.0)
        reward_on = cfg.fight_reward != 0.0
        score_on = cfg.fight_score != 0.0
        win_on = cfg.win_score != 0.0
        fight_on = reward_on or score_on or win_on
        pending = np.full((N, P), -1, np.int64) if reward_on else None
        pending_turn = np.full((N, P), -1, np.int64) if reward_on else None
        pending_score = np.full((N, P), -1, np.int64) if score_on else None
        pending_score_turn = np.full((N, P), -1, np.int64) if score_on else None
        pending_win = np.full((N, P), -1, np.int64) if win_on else None
        pending_win_turn = np.full((N, P), -1, np.int64) if win_on else None
        fight_sum, fight_turns, score_sum = 0.0, 0, 0.0
        win_sum, win_turns = 0.0, 0
        with torch.no_grad():
            for t in range(T):
                env.observe(b.obs, b.mask, b.seat, b.kind)
                if score_on:
                    env.fight_damage(b.fight_dealt, b.fight_taken)
                if reward_on:
                    # A learner's next view carries the fight after its
                    # pending turn, perhaps several steps after that turn.
                    for e in np.nonzero(b.kind == LEARNER)[0]:
                        s = int(b.seat[e])
                        if pending[e, s] < 0 or b.obs[e, TURN] <= pending_turn[e, s]:
                            continue
                        term = credit_fight(
                            rews, pending, int(e), s,
                            fight_outcome(b.obs[e, WON_LAST_COMBAT],
                                          b.obs[e, LOST_LAST_COMBAT]),
                            cfg.fight_reward)
                        if term is not None:
                            fight_sum += term
                if score_on:
                    for e in np.nonzero(b.kind == LEARNER)[0]:
                        s = int(b.seat[e])
                        if (pending_score[e, s] < 0
                                or b.obs[e, TURN] <= pending_score_turn[e, s]):
                            continue
                        term = credit_fight_score(
                            rews, pending_score, pending_score_turn, int(e), s,
                            b.fight_dealt[e, s], b.fight_taken[e, s],
                            score_weight, cfg.fight_score_from)
                        if term is not None:
                            score_sum += term
                if win_on:
                    # The result comes from the won/lost scalars, not the
                    # damage: a won ghost fight deals 0, and a loss can take
                    # 0, so damage cannot tell a win from a tie or a loss.
                    for e in np.nonzero(b.kind == LEARNER)[0]:
                        s = int(b.seat[e])
                        if (pending_win[e, s] < 0
                                or b.obs[e, TURN] <= pending_win_turn[e, s]):
                            continue
                        term = credit_win_score(
                            rews, pending_win, pending_win_turn, int(e), s,
                            win_score_outcome(b.obs[e, WON_LAST_COMBAT],
                                              b.obs[e, LOST_LAST_COMBAT]),
                            win_weight, cfg.win_score_from)
                        if term is not None:
                            win_sum += term
                o = torch.from_numpy(b.obs).to(dev, non_blocking=True)
                m = torch.from_numpy(b.mask).to(dev, non_blocking=True)
                is_learner = torch.from_numpy(b.kind == LEARNER).to(dev)
                m = force_mask(o, m)
                bonus = forced = comp = None
                target = np.full(N, -1, np.int64)
                if cfg.overlay_comps:
                    env.overlay(b.forced, b.bonus, b.flags)
                    rows = np.arange(N)
                    raw = b.bonus[rows, b.seat].copy()
                    forced_np = b.forced[rows, b.seat].copy()
                    comp_idx[t] = b.flags[rows, b.seat, F_COMP] - 1
                    comp_active[t] = comp_idx[t] >= 0
                    comp = comp_onehot(comp_idx[t], COMP_COUNT, dev)
                    # A PPO forced press (Dark Gift, level) wins over the
                    # overlay for that row.
                    blocked = (m != torch.from_numpy(b.mask).to(dev)).any(1)
                    blocked = blocked.cpu().numpy()
                    forced_np[blocked] = -1
                    raw[blocked] = 0.0
                    if wheel > 0.0:
                        # Forced moves cannot be scaled: they are on until the
                        # end of the fade, and gone after it.
                        bonus = torch.from_numpy(
                            (raw * np.float32(wheel)).astype(np.float32)).to(dev)
                        forced = torch.from_numpy(forced_np).to(dev)
                        forced_rows[t] = forced_np >= 0
                        bonus_store.put(t * N + rows, raw)
                        # The aux target: the forced action, else the best
                        # payoff/filler buy, Tavern spell, pick or play that
                        # is legal and positively bonused while a comp is on.
                        target = forced_np.copy()
                        pos = (b.mask & AUX_ACTIONS[None, :] & (raw > 0)
                               & ~blocked[:, None])
                        cand = (target < 0) & comp_active[t] & pos.any(1)
                        score = np.where(pos, raw, -np.inf)
                        target[cand] = score[cand].argmax(1)
                aux_target[t] = target
                a, lp, v, _ = act(net, o, m, bonus=bonus, forced=forced, comp=comp)
                sn = ~is_learner
                if sn.any():
                    sa, _, _, _ = act(
                        snap, o[sn], m[sn],
                        bonus=bonus[sn] if bonus is not None else None,
                        forced=forced[sn] if forced is not None else None,
                        comp=comp[sn] if comp is not None else None,
                    )
                    a[sn] = sa
                obs[t], masks[t], acts[t], logps[t], vals[t] = o, m, a, lp, v
                rews[t] = -cost_of[kind_of[a]]
                if low_pen > 0:
                    ends_turn = (a == END_TURN) | (o[:, ACTIONS_LEFT] <= 1)
                    turn_i = o[:, bgpy.SCALAR_TURN].long().clamp(1, len(curve)) - 1
                    behind = (curve[turn_i] - o[:, bgpy.SCALAR_TIER]).clamp(min=0)
                    rews[t] -= low_pen * behind * ends_turn
                if cfg.deadline_penalty:
                    ends_turn = (a == END_TURN) | (o[:, ACTIONS_LEFT] <= 1)
                    turn = o[:, bgpy.SCALAR_TURN]
                    tier = o[:, bgpy.SCALAR_TIER]
                    short2 = (2 - tier).clamp(min=0) * (turn >= 2)
                    need = late_need[turn.long().clamp(1, len(late_need)) - 1]
                    short_late = (need - tier.clamp(min=2)).clamp(min=0)
                    w2 = cfg.deadline_turn2_floor + dl_scale * (cfg.deadline_turn2 - cfg.deadline_turn2_floor)
                    pen = w2 * short2 + dl_scale * cfg.deadline_later * short_late
                    rews[t] -= pen * ends_turn
                learn[t] = is_learner
                seats[t] = torch.from_numpy(b.seat).to(dev)
                le = np.nonzero(b.kind == LEARNER)[0]
                last[le, b.seat[le]] = t
                if fight_on:
                    ended_turn = ((a == END_TURN) | (o[:, ACTIONS_LEFT] <= 1)).cpu().numpy()
                    for e in np.nonzero((b.kind == LEARNER) & ended_turn)[0]:
                        s = int(b.seat[e])
                        turn = int(b.obs[e, TURN])
                        if reward_on:
                            pending[e, s] = t
                            pending_turn[e, s] = turn
                        if score_on:
                            pending_score[e, s] = t
                            pending_score_turn[e, s] = turn
                        if win_on:
                            pending_win[e, s] = t
                            pending_win_turn[e, s] = turn
                            # The per-turn log divides by the turns the term
                            # can pay on, not the early ones it skips.
                            if turn >= cfg.win_score_from:
                                win_turns += 1
                        fight_turns += 1
                kinds = b.kinds.copy()
                env.step(a.cpu().numpy(), b.places, b.tier_turns, b.tribes, b.reset)
                if score_on and b.places.any():
                    # A game that ended has already been replaced: the engine
                    # keeps the knockout's uncapped damage, so it is still here.
                    env.fight_damage(b.fight_dealt, b.fight_taken)
                for e, s in zip(*np.nonzero(b.places)):
                    if kinds[e, s] == LEARNER and last[e, s] >= 0:
                        rews[last[e, s], e] += reward(
                            b.places[e, s], cfg.first_bonus, cfg.second_bonus)
                        dones[last[e, s], e] = 1.0
                        if reward_on:
                            term = credit_fight(
                                rews, pending, e, s,
                                place_fight_outcome(int(b.places[e, s])),
                                cfg.fight_reward)
                            if term is not None:
                                fight_sum += term
                        if score_on:
                            # A knockout counts the raw damage taken (or dealt,
                            # for 1st place) in the fight that decided it.
                            term = credit_fight_score(
                                rews, pending_score, pending_score_turn, e, s,
                                b.fight_dealt[e, s], b.fight_taken[e, s],
                                score_weight, cfg.fight_score_from)
                            if term is not None:
                                score_sum += term
                        if win_on:
                            # A knockout is a loss, so only 1st place pays.
                            term = credit_win_score(
                                rews, pending_win, pending_win_turn, e, s,
                                place_win_score_outcome(int(b.places[e, s])),
                                win_weight, cfg.win_score_from)
                            if term is not None:
                                win_sum += term
                if b.reset.any():
                    r = np.nonzero(b.reset)[0]
                    last[r] = -1
                    games += len(r)
                    env.kinds(b.kinds)

            # bootstrap from every live learner seat's current view
            all_obs = np.zeros((N, P, bgpy.OBS_SIZE), np.float32)
            all_comp = np.full((N, P), -1, np.int64)
            live = np.zeros((N, P), bool)
            env.observe_all(all_obs, live)
            if score_on:
                env.fight_damage(b.fight_dealt, b.fight_taken)
            if reward_on:
                # A turn that ended on the rollout's last step resolves from
                # this bootstrap view, before the advantages are computed.
                for e, s in zip(*np.nonzero(live)):
                    s = int(s)
                    # The combat scalars only change when the round's fight
                    # has happened; before that they are the previous fight.
                    if pending[e, s] < 0 or all_obs[e, s, TURN] <= pending_turn[e, s]:
                        continue
                    term = credit_fight(
                        rews, pending, int(e), s,
                        fight_outcome(all_obs[e, s, WON_LAST_COMBAT],
                                      all_obs[e, s, LOST_LAST_COMBAT]),
                        cfg.fight_reward)
                    if term is not None:
                        fight_sum += term
                unresolved = int((pending >= 0).sum())
                if unresolved:
                    print(f"fight reward: {unresolved} learner turns unresolved at the "
                          "rollout boundary, credited 0")
                pending.fill(-1)
            if score_on:
                for e, s in zip(*np.nonzero(live)):
                    s = int(s)
                    if (pending_score[e, s] < 0
                            or all_obs[e, s, TURN] <= pending_score_turn[e, s]):
                        continue
                    term = credit_fight_score(
                        rews, pending_score, pending_score_turn, int(e), s,
                        b.fight_dealt[e, s], b.fight_taken[e, s],
                        score_weight, cfg.fight_score_from)
                    if term is not None:
                        score_sum += term
                unresolved = int((pending_score >= 0).sum())
                if unresolved:
                    print(f"fight score: {unresolved} learner turns unresolved at the "
                          "rollout boundary, credited 0")
                pending_score.fill(-1)
            if win_on:
                for e, s in zip(*np.nonzero(live)):
                    s = int(s)
                    if (pending_win[e, s] < 0
                            or all_obs[e, s, TURN] <= pending_win_turn[e, s]):
                        continue
                    term = credit_win_score(
                        rews, pending_win, pending_win_turn, int(e), s,
                        win_score_outcome(all_obs[e, s, WON_LAST_COMBAT],
                                          all_obs[e, s, LOST_LAST_COMBAT]),
                        win_weight, cfg.win_score_from)
                    if term is not None:
                        win_sum += term
                unresolved = int((pending_win >= 0).sum())
                if unresolved:
                    print(f"win score: {unresolved} learner turns unresolved at the "
                          "rollout boundary, credited 0")
                pending_win.fill(-1)
            if cfg.overlay_comps:
                env.overlay_comp(all_comp)
            flat_comp = all_comp.reshape(-1)
            # in chunks: one pass over all N * P seats is the run's biggest activation peak
            flat = torch.from_numpy(all_obs.reshape(N * P, -1))
            if cfg.obs_fp16:
                flat = flat.half()  # the same rounding the rollout buffer gets
            boot = torch.cat([net(flat[i:i + 1024].to(dev).float(),
                                 comp=comp_onehot(flat_comp[i:i + 1024], COMP_COUNT, dev))[1]
                              for i in range(0, N * P, 1024)])
            next_v = (boot.reshape(N, P) * torch.from_numpy(live).to(dev)).clone()
            next_a = torch.zeros((N, P), device=dev)
            adv = torch.zeros((T, N), device=dev)
            ar = torch.arange(N, device=dev)
            # an action that ends the seat's turn moves time on by one turn;
            # gamma and lambda both count turns, not actions
            ends = (acts == END_TURN) | (obs[:, :, ACTIONS_LEFT] <= 1)
            gammas = torch.where(ends, cfg.gamma, 1.0)
            lams = torch.where(ends, cfg.lam, 1.0)
            for t in reversed(range(T)):
                s = seats[t]
                nv, na = next_v[ar, s], next_a[ar, s]
                nd = 1.0 - dones[t]
                g = gammas[t]
                delta = rews[t] + g * nv * nd - vals[t]
                at = delta + g * lams[t] * nd * na
                adv[t] = at
                lm = learn[t]
                next_v[ar[lm], s[lm]] = vals[t][lm]
                next_a[ar[lm], s[lm]] = at[lm]
            rets = adv + vals
        t_roll = time.time() - t0

        sel = learn.reshape(-1)
        sel_np = sel.cpu().numpy()
        f_obs = obs.reshape(T * N, -1)[sel]
        f_mask = masks.reshape(T * N, -1)[sel]
        f_act = acts.reshape(-1)[sel]
        f_logp = logps.reshape(-1)[sel]
        f_adv = adv.reshape(-1)[sel]
        f_ret = rets.reshape(-1)[sel]
        f_val = vals.reshape(-1)[sel]
        f_prior = None
        if (buy_prior_model is not None and buy_prior_weight > 0.0
                and update >= cfg.value_warmup):
            with torch.no_grad():
                f_prior = bp.prior_logits_for(
                    f_obs.detach().float().cpu().numpy(), buy_prior_pidx,
                    buy_prior_model, buy_prior_ck, "cpu",
                    SHOP_COLS, BOARD_COLS, TURN,
                    bgpy.SLOT_PRESENT, bgpy.SLOT_CARD_ID).to(dev)
        f_comp = comp_onehot(comp_idx.reshape(-1)[sel_np], COMP_COUNT, dev)
        f_forced = torch.from_numpy(forced_rows.reshape(-1)[sel_np]).to(dev)
        f_aux = torch.from_numpy(aux_target.reshape(-1)[sel_np]).to(dev)
        f_bonus = bonus_store.get(sel_np, dev) if cfg.overlay_comps and wheel > 0.0 else None
        n = f_obs.shape[0]
        if cfg.warmup:
            for g in opt.param_groups:
                g["lr"] = cfg.lr * min(1.0, (update + 1) / cfg.warmup)
        kls, clips, ents, plains, vls, auxs = [], [], [], [], [], []
        move_idx = (torch.as_tensor(MOVE_IDX, dtype=torch.long, device=dev)
                    if cfg.entropy_moves_as_one else None)
        for _ in range(cfg.epochs):
            perm = torch.randperm(n, device=dev)
            for i in range(0, n, cfg.minibatch):
                idx = perm[i:i + cfg.minibatch]
                # the learner's step count varies, so the last minibatch can be tiny:
                # its advantage std is noise, or NaN for one sample (which poisoned a run)
                if idx.numel() < 256:
                    continue
                # --micro-batch: the same minibatch in pieces with the gradients added
                # up, so the update is identical and the activation peak smaller
                ad_all = f_adv[idx]
                ad_mean, ad_std = ad_all.mean(), ad_all.std() + 1e-8
                m = idx.numel()
                step = cfg.micro_batch or m
                demo_rows = None
                if (demo is not None
                        and (demo_weight > 0.0 or demo_kind_weight > 0.0)
                        and update >= cfg.value_warmup):
                    # Fresh every minibatch; the demo generator is separate so
                    # the auxiliary sample does not move the PPO RNG.
                    demo_rows = demo.train[torch.randint(
                        len(demo.train), (m,), generator=demo_gen)]
                opt.zero_grad()
                kl_s = clip_s = ent_s = plain_s = vl_s = aux_s = 0.0
                for j in range(0, m, step):
                    sub = idx[j:j + step]
                    w = sub.numel() / m
                    logits, v = net(
                        f_obs[sub].float(),
                        comp=f_comp[sub] if f_comp is not None else None,
                    )
                    # The policy that acted is the network plus the stored
                    # bonus; the ratio and the KL use it. Entropy and the aux
                    # cross-entropy are the network's own policy, which is
                    # what survives when the wheels are gone.
                    act_logits = logits if f_bonus is None else logits + f_bonus[sub]
                    dist = torch.distributions.Categorical(logits=masked(act_logits, f_mask[sub]))
                    own_dist = (dist if f_bonus is None
                                else torch.distributions.Categorical(
                                    logits=masked(logits, f_mask[sub])))
                    lp = dist.log_prob(f_act[sub])
                    ratio = (lp - f_logp[sub]).exp()
                    ad = (f_adv[sub] - ad_mean) / ad_std
                    pg_i = torch.max(-ad * ratio, -ad * ratio.clamp(1 - cfg.clip, 1 + cfg.clip))
                    # An overlay-forced action has no policy gradient: the
                    # aux loss teaches that choice instead.
                    keep = ~f_forced[sub]
                    pg = (pg_i * keep).sum() / keep.sum().clamp(min=1)
                    vl = 0.5 * ((v - f_ret[sub]) ** 2).mean()
                    ent = entropy_term(own_dist, cfg.entropy_moves_as_one, move_idx)
                    plain_ent = (own_dist.entropy().mean()
                                 if cfg.entropy_moves_as_one else None)
                    target = f_aux[sub]
                    valid = target >= 0
                    aux = (-F.log_softmax(own_dist.logits[valid], -1)
                           .gather(1, target[valid, None]).mean()
                           if valid.any() else logits.sum() * 0.0)
                    if update < cfg.value_warmup:
                        loss = cfg.vf * vl
                    else:
                        loss = pg + cfg.vf * vl - cfg.ent * ent + aux_w * aux
                    if demo_rows is not None:
                        dsub = demo_rows[j:j + sub.numel()]
                        d_obs = demo.obs[dsub].to(dev)
                        d_mask = demo.mask[dsub].to(dev)
                        d_action = demo.action[dsub].to(dev)
                        if demo_weight > 0.0:
                            loss = add_demo_loss(
                                loss, net, d_obs, d_mask, d_action, demo_weight)
                        if demo_kind_weight > 0.0:
                            loss = add_demo_kind_loss(
                                loss, net, d_obs, d_mask, d_action, demo_kind_weight)
                    if f_prior is not None:
                        loss = add_buy_prior_loss(
                            loss, logits, f_mask[sub], f_prior[sub],
                            buy_prior_weight)
                    (loss * w).backward()
                    with torch.no_grad():
                        kl_s += w * ((ratio - 1) - (lp - f_logp[sub])).mean().item()
                        clip_s += w * ((ratio - 1).abs() > cfg.clip).float().mean().item()
                        ent_s += w * ent.item()
                        if cfg.entropy_moves_as_one:
                            plain_s += w * plain_ent.item()
                        vl_s += w * vl.item()
                        aux_s += w * aux.item()
                torch.nn.utils.clip_grad_norm_(net.parameters(), 0.5)
                opt.step()
                kls.append(kl_s)
                clips.append(clip_s)
                ents.append(ent_s)
                # Off, the term is the plain entropy already: no second sync.
                plains.append(plain_s if cfg.entropy_moves_as_one else ent_s)
                vls.append(vl_s)
                auxs.append(aux_s)
        update += 1

        # behaviour, from this rollout's learner samples
        with torch.no_grad():
            k = kind_of[f_act]
            counts = torch.bincount(k, minlength=len(KINDS)).float()
            turn = f_obs[:, bgpy.SCALAR_TURN]
            tier = f_obs[:, bgpy.SCALAR_TIER]
            health = f_obs[:, bgpy.SCALAR_HEALTH]
            t6, t10 = tier[turn == 6], tier[turn == 10]
            is_buy = (k == 0) | (k == 1)
            n_moves = (k == MOVE).sum().item()
            buys = f_act[is_buy]
            card_ids = f_obs[is_buy, SHOP_START + buys * bgpy.SLOT_FEATURES + bgpy.SLOT_CARD_ID].long() - 1
            card_ids = card_ids[card_ids >= 0]
            buy_counts = torch.bincount(card_ids, minlength=len(names))
            ev = 1 - (f_ret - f_val).var() / (f_ret.var() + 1e-8)
            n_ends = ends.reshape(-1)[sel].sum().item()
            term = dones.reshape(-1)[sel] > 0
            stats = {
                "entropy": np.mean(ents), "approx_kl": np.mean(kls), "clipfrac": np.mean(clips),
                "value_loss": np.mean(vls), "explained_var": ev.item(),
                "steps_per_s": T * N / t_roll,
                "actions_per_turn": n / max(n_ends, 1),
                "tier_t6": t6.mean().item() if len(t6) else float("nan"),
                "tier_t10": t10.mean().item() if len(t10) else float("nan"),
                "health_end": health[term].mean().item() if term.any() else float("nan"),
                "aux_loss": float(np.mean(auxs)) if auxs else 0.0,
                "wheels_b": wheel * cfg.overlay_bonus,
                "wheels_lambda": aux_w,
                "comp_active": float(comp_active.reshape(-1)[sel_np].mean()),
                "entropy_plain": np.mean(plains),
                "moves_per_turn": n_moves / max(n_ends, 1),
                "fight_reward_per_turn": fight_sum / fight_turns if fight_turns else 0.0,
                "fight_score_per_turn": score_sum / fight_turns if fight_turns else 0.0,
                "win_score_per_turn": win_sum / win_turns if win_turns else 0.0,
                **{f"act_{KINDS[i]}": (counts[i] / counts.sum()).item() for i in range(len(KINDS))},
            }
            top = buy_counts.argsort(descending=True)[:5]
            total = max(buy_counts.sum().item(), 1)
            top_buys = " ".join(f"{names[i]}:{buy_counts[i].item() / total:.0%}" for i in top.tolist())
        for key, val in stats.items():
            tb.add_scalar(f"train/{key}", val, update)

        if update % cfg.snapshot_every == 0:
            pool.append(copy.deepcopy({k: v.cpu() for k, v in net.state_dict().items()}))
            pool = pool[-cfg.snapshot_pool:]

        minutes = t_total + (time.time() - start) / 60
        done = minutes >= cfg.minutes
        if update % cfg.eval_every == 0 or done:
            net.eval()
            demo_vals = (demo_agreement(net, demo, dev,
                                        with_kind=bool(cfg.demo_kind_weight))
                         if demo is not None else {})
            seed = 10_000 + update * 10
            std_result = evaluate(
                net, cfg.eval_games, [1, 0, 7, 0, 0, 0], dev, seed,
                buy_prior=buy_prior_data)
            if buy_prior_data is None:
                vs_std, rec_std = std_result
                agree_std = None
            else:
                vs_std, rec_std, agree_std = std_result
            vs_rush, rec_rush = evaluate(
                net, cfg.eval_games, [1, 0, 0, 7, 0, 0], dev, seed + 1)
            v4_std, rec_4s = evaluate(
                net, cfg.eval_games // 2, [4, 0, 4, 0, 0, 0], dev, seed + 2)
            v4_rush, rec_4r = evaluate(
                net, cfg.eval_games // 2, [4, 0, 0, 4, 0, 0], dev, seed + 3)
            v4_mix, rec_4m = evaluate(
                net, cfg.eval_games // 2, [4, 0, 2, 2, 0, 0], dev, seed + 4)
            snap.load_state_dict(pool[0])  # the oldest snapshot kept
            vs_snap = evaluate_vs(net, snap, cfg.eval_games // 2, dev, seed + 5)
            # One learner against 7 real seats: does it beat real boards?
            vs_real = float("nan")
            rec_real = None
            if real is not None:
                vs_real, rec_real = evaluate(
                    net, cfg.eval_games, [1, 0, 0, 0, 0, 7],
                    dev, seed + 6, real)
            # The same game at the training lag mix, for the share and the
            # place at each lag. The vs7 real number above stays at lag 0.
            lag_vals = {}
            if REAL_LAG_MIX and real is not None:
                _, rec_lag = evaluate(
                    net, cfg.eval_games, [1, 0, 0, 0, 0, 7], dev,
                    seed + 60, real, real_lag_mix=REAL_LAG_MIX)
                lag_places = np.array([r[0] for r in rec_lag], dtype=np.float64)
                lag_drawn = np.array([r[3]["real_lag"] for r in rec_lag], dtype=np.int64)
                lag_vals["place_vs_real_mix"] = float(lag_places.mean())
                for lag in LAG_MIX_LAGS:
                    at = lag_drawn == lag
                    lag_vals[f"real_lag_{lag}_share"] = float(at.mean())
                    lag_vals[f"real_lag_{lag}_place"] = (
                        float(lag_places[at].mean()) if at.any() else float("nan"))
            # Scenario starts, wheels off, for every comp and pooled. Each is
            # one learner against seven real seats, the body sold measured
            # against the board the scenario started with.
            scen_vals = {}
            if SCENARIO_LABELS and rec_real is not None:
                for k, (label, snames, specs) in enumerate(SCENARIO_LABELS):
                    _, rec_s = evaluate(
                        net, cfg.eval_games, [1, 0, 0, 0, 0, 7], dev,
                        seed + 20 + 3 * k, real, scenario_names=snames)
                    for metric, val in summarize_scenario(rec_s, specs).items():
                        scen_vals[f"scen_{scenario_slug(label)}_{metric}"] = val
            # Era starts from the holdout split, wheels off: one learner
            # against seven real seats, every game an Era start.
            era_vals = {}
            if cfg.era_starts:
                _, rec_era = evaluate(
                    net, cfg.eval_games, [1, 0, 0, 0, 0, 7], dev,
                    seed + 50, real, era_starts=cfg.era_starts,
                    era_split="holdout", era_start_lag=ERA_START_LAG)
                for metric, val in summarize_era(rec_era).items():
                    era_vals[f"era_{metric}"] = val
            net.train()

            rep = summarize_records(rec_std + rec_rush + rec_4s + rec_4r + rec_4m)
            # The comp line is the wheels-off view: the real-board eval when
            # there is one, so --real-lobby-filter is the same as the score.
            comp_rep = summarize_comps(
                rec_real if rec_real is not None else rec_std + rec_rush + rec_4s + rec_4r + rec_4m,
                COMP_SPECS,
            )
            prior_vals = {}
            if buy_prior_model is not None:
                hits, total = agree_std
                prior_vals["buy_prior_top1"] = (hits / total if total
                                                else float("nan"))
            eval_vals = {
                "place_vs_std": vs_std,
                "place_vs_rush": vs_rush,
                "place_4v4_std": v4_std,
                "place_4v4_rush": v4_rush,
                "place_4v4_mix": v4_mix,
                "place_vs_snapshot": vs_snap,
                "place_vs_real": vs_real,
                **rep,
                **comp_rep,
                **scen_vals,
                **era_vals,
                **lag_vals,
                **demo_vals,
                **prior_vals,
            }
            for key, val in eval_vals.items():
                tb.add_scalar(f"eval/{key}", val, update)

            log.writerow([
                update, f"{minutes:.1f}", games,
                *[f"{eval_vals[k]:.3f}" for k in eval_keys],
                *[f"{stats[k]:.4g}" for k in TRAIN_KEYS],
                *[f"{stats[f'act_{k}']:.3f}" for k in KINDS],
                top_buys,
                *[f"{stats[k]:.4g}" for k in EXTRA_KEYS],
            ])
            csv_file.flush()
            score = vs_real if best_by == "real" else 0.5 * (vs_std + vs_rush)
            ck = {"net": net.state_dict(), "opt": opt.state_dict(), "pool": pool, "update": update,
                  "best": best, "best_by": best_by, "t_total": minutes, "games": games,
                  "cfg": vars(cfg), "model": cfg.model, "model_opts": opts,
                  "card_names": names, "hero_names": hero_names,
                  "trinket_names": trinket_names}
            if score < best:
                best = score
                ck["best"] = best
                torch.save(ck, out / "best.pt")
            torch.save(ck, out / "ckpt.pt")
            if minutes - last_keep >= cfg.keep_every:
                (out / "keep").mkdir(exist_ok=True)
                torch.save({"net": net.state_dict(), "update": update, "model": cfg.model,
                            "model_opts": opts, "card_names": names, "hero_names": hero_names,
                            "trinket_names": trinket_names},
                           out / "keep" / f"net_{update}.pt")
                last_keep = minutes
            # plateau: the rolling score hasn't improved for a while and it no
            # longer beats its oldest snapshot
            history.append((minutes, score, vs_snap))
            recent = [h for h in history if h[0] > minutes - 30]
            rolling = sum(h[1] for h in recent) / len(recent)
            if rolling < flat_best - 0.03:
                flat_best, flat_since = rolling, minutes
            snap_recent = sum(h[2] for h in recent) / len(recent)
            if cfg.stop_flat and minutes - flat_since >= cfg.stop_flat and snap_recent >= 4.35:
                print(f"stopping: no gain for {minutes - flat_since:.0f} min and even with the "
                      f"oldest snapshot ({snap_recent:.2f})", flush=True)
                done = True
            scen_txt = ""
            if scen_vals:
                parts = []
                for label, _, _ in SCENARIO_LABELS:
                    slug = scenario_slug(label)
                    short = label.split(" - ")[-1].replace(" ", "")
                    v = {m: scen_vals[f"scen_{slug}_{m}"] for m in scen_metrics}
                    parts.append(f"{short} {v['place']:.2f}/{v['top4']:.0%}/"
                                 f"{v['first']:.0%}/{v['end']:.0%}/{v['body_sold']:.0%}")
                scen_txt = "  scen(" + " ".join(parts) + ")"
            era_txt = ""
            if era_vals:
                era_txt = (f"  era(place {era_vals['era_place']:.2f}"
                           f"/{era_vals['era_top4']:.0%}/{era_vals['era_first']:.0%}"
                           f"/body {era_vals['era_body_kept']:.0%})")
            lag_txt = ""
            if lag_vals:
                parts = [
                    f"{lag} {lag_vals[f'real_lag_{lag}_share']:.0%}"
                    f"@{lag_vals[f'real_lag_{lag}_place']:.2f}"
                    for lag in LAG_MIX_LAGS
                ]
                lag_txt = (f"  lagmix(all {lag_vals['place_vs_real_mix']:.2f}; "
                           + " ".join(parts) + ")")
            demo_txt = ""
            if demo_vals:
                parts = [f"top1 {demo_vals['demo_top1']:.1%}"]
                if cfg.demo_kind_weight:
                    parts.append(f"kind {demo_vals['demo_kind_top1']:.1%}")
                parts += [f"buy {demo_vals['demo_top1_buy']:.1%}",
                          f"train {demo_vals['demo_top1_train']:.1%}"]
                if cfg.demo_kind_weight:
                    parts.append(
                        f"kindtrain {demo_vals['demo_kind_top1_train']:.1%}")
                demo_txt = "  demo(" + " ".join(parts) + ")"
            prior_txt = ""
            if prior_vals:
                prior_txt = f"  buy_prior {prior_vals['buy_prior_top1']:.1%}"
            gate_txt = ""
            gates = text_gates(net)
            if gates:
                gate_txt = "  gate(" + " ".join(
                    f"{name} {value:.4f}" for name, value in gates) + ")"
            print(
                f"update {update:5d}  {minutes:5.1f} min  {games:7d} games  "
                f"vs7(std {vs_std:.2f} rush {vs_rush:.2f} real {vs_real:.2f})  "
                f"4v4(std {v4_std:.2f} rush {v4_rush:.2f} mix {v4_mix:.2f} snap {vs_snap:.2f})  "
                f"t3/4/5 {rep['turn_t3']:.1f}/{rep['turn_t4']:.1f}/{rep['turn_t5']:.1f}  "
                f"crv(std {rep['curve_standard']:.0%} rush {rep['curve_rush']:.0%} "
                f"econ {rep['curve_econ_rush']:.0%} slow {rep['curve_slow']:.0%})  "
                "trb(" + " ".join(
                    f"{t[:2]} {rep[f'tribe_{t}']:.0%}@{rep[f'place_{t}']:.2f}"
                    for t in REPORT_GROUPS) + ")  "
                f"comp(active {comp_rep['comp_active_share']:.0%} "
                f"end {comp_rep['comp_end_share']:.0%} "
                f"shell {comp_rep['shell_rate']:.0%} first {comp_rep['first_share']:.0%})  "
                f"wheels(B {stats['wheels_b']:.2f} lam {stats['wheels_lambda']:.2f} "
                f"aux {stats['aux_loss']:.3f} active {stats['comp_active']:.0%})  "
                f"fight(rew {stats['fight_reward_per_turn']:.3f} "
                f"score {stats['fight_score_per_turn']:.3f} "
                f"win {stats['win_score_per_turn']:.3f})  "
                f"ent {stats['entropy']:.2f}  {stats['steps_per_s']:.0f} steps/s  "
                f"buys {top_buys}{prior_txt}{demo_txt}{scen_txt}{era_txt}{lag_txt}{gate_txt}",
                flush=True,
            )
        if done:
            break


if __name__ == "__main__":
    main()
