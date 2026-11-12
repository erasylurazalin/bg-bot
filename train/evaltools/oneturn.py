"""One-turn and whole-game fine-tunes on the r1111 network.

--mode turn (default): every game starts from Era's logged turn 7..11
position, the episode is the learner's first turn, and the reward is the
fight that follows (+1 win, 0 tie, -1 loss). This is the original diagnostic.

--mode game: normal games from turn 1, one learner against seven real seats.
The episode is the learner's whole game. Rewards are optional and add up:
--fight-reward W at every turn end (+W win, 0 tie, -W loss) and --place-reward
P * (4.5 - place) / 3.5 at the learner's last action. Move actions cost 0.002.
Returns are Monte Carlo with --gamma applied once per turn end. --no-moves
removes move actions whenever another action is legal.

    .venv/bin/python train/evaltools/oneturn.py --name smoke --minutes 3 \
        --init train/runs/r1111-lagmix/best.pt --vram-gb 6

Outputs go to train/runs/oneturn/<name>/: log.tsv, tests.tsv, final.pt and
report.md. final.pt keeps the --init checkpoint's dict and replaces its net,
so train/fixed_eval.py can load it.
"""

from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

import numpy as np
import torch
from torch.distributions import Categorical

ROOT = Path(__file__).resolve().parents[2]
TRAIN = ROOT / "train"
sys.path.insert(0, str(TRAIN))
sys.path.insert(0, str(ROOT / "train" / "evaltools"))

import bgpy
import buy_prior as bp
import ppo
import real_boards
from fixed_eval import check_boards, load_ckpt
from model import masked

FROZEN = ROOT / "boards" / "frozen" / "real-2026-11-10"
FROZEN_ACTIONS = ROOT / "boards" / "frozen" / "actions-2026-11-10"
OUT_ROOT = TRAIN / "runs" / "oneturn"

LINEUP = [1, 0, 0, 0, 0, 7]
OLD_BOARDS = "keep"
ERA_TURN_MIN = 7
ERA_TURN_MAX = 11
TEST_CHUNK = 512
TEST_CHUNKS = 2
TEST_SEED_BASE = 20261111
TARGET_SAMPLES = 32768
MINIBATCH = 8192
MICRO_BATCH = 2048
EPOCHS = 3
CLIP = 0.2
VF = 0.5
VALUE_WARMUP = 10
GRAD_CLIP = 0.5
ACTION_COST = 0.002
END_TURN = bgpy.END_TURN
ACTIONS_LEFT = bgpy.SCALAR_ACTIONS_LEFT
GOLD = bgpy.SCALAR_GOLD
WON = bgpy.SCALAR_WON_LAST_COMBAT
LOST = bgpy.SCALAR_LOST_LAST_COMBAT
TURN = bgpy.SCALAR_TURN
TIER = bgpy.SCALAR_TIER
HEALTH = bgpy.SCALAR_HEALTH
ARMOR = bgpy.SCALAR_ARMOR
KINDS = [name for name, _ in bgpy.ACTION_KINDS]
KIND_OF = np.repeat(np.arange(len(KINDS)),
                    [n for _, n in bgpy.ACTION_KINDS]).astype(np.int64)
COST_OF = np.where(np.array(KINDS)[KIND_OF] == "move", ACTION_COST, 0.0)
MOVE_IDX = np.nonzero(KIND_OF == KINDS.index("move"))[0]
assert len(KIND_OF) == bgpy.N_ACTIONS
REPORT_TURNS = tuple(range(7, 15))
TIER_TURNS = (2, 5, 7, 8, 10)
FIGHT_BANDS = ("1_6", "7_9", "10_plus")
LATE_METRICS = ("actions", "roll", "buy", "buy_spell", "sell", "play",
                "move")
DEADLINE_CURVE = (0.0, 0.0, 0.0, 0.0, 3.0, 3.0, 4.0, 5.0)
DEADLINE_TURN2 = 0.30
DEADLINE_LATER = 0.15
TEST_PRIOR = TRAIN / "runs" / "buy-prior" / "buy_prior_all.pt"
LAG_MIX = [(0, 1), (1, 1), (2, 1)]
CARD_TARGETS = (
    "Tavern Coin", "Wealthy Bounty", "Corrupted Coin", "Careful Investment",
    "Leaf Through the Pages", "Fandral's Fortune", "Greedy Conniver",
    "Eternal Knight", "Nightmare Par-tea Guest", "Mighty Dragonbreath",
    "Misplaced Tea Set",
)
SF = bgpy.SLOT_FEATURES
SHOP_COLS = [bgpy.SHOP_START + i * SF for i in range(bgpy.SHOP)] + [
    bgpy.SHOP_SPELL_START]
BOARD_COLS = [bgpy.BOARD_START + i * SF for i in range(bgpy.BOARD)]
BUY0 = sum(n for name, n in bgpy.ACTION_KINDS[:KINDS.index("buy")])
BUY_ACTIONS = list(range(BUY0, BUY0 + bgpy.SHOP + 1))
ROLL_ACTION = sum(n for name, n in bgpy.ACTION_KINDS[:KINDS.index("roll")])
assert [x[1] for x in bgpy.ACTION_KINDS[:2]] == [bgpy.SHOP, 1]
assert KINDS[KIND_OF[ROLL_ACTION]] == "roll"


def deadline_penalty(turn, tier):
    """train/ppo.py's Era deadline penalty at a turn-ending action."""
    turn = max(1, int(turn))
    tier = int(tier)
    short2 = max(0, 2 - tier) if turn >= 2 else 0
    need = DEADLINE_CURVE[min(turn, 8) - 1]
    short_late = max(0.0, need - max(tier, 2))
    return DEADLINE_TURN2 * short2 + DEADLINE_LATER * short_late


def fight_band(turn):
    if turn <= 6:
        return "1_6"
    if turn <= 9:
        return "7_9"
    return "10_plus"


def no_moves_mask(mask):
    """Clear move actions on rows that have another legal action."""
    if mask.numel() == 0:
        return mask
    idx = torch.as_tensor(MOVE_IDX, dtype=torch.long, device=mask.device)
    other = mask.clone()
    other[:, idx] = False
    keep = ~other.any(dim=1, keepdim=True)
    out = mask.clone()
    out[:, idx] = out[:, idx] & keep
    return out


def load_prior(path, dev):
    ck = torch.load(path, map_location="cpu", weights_only=False)
    sd = ck["state_dict"]
    model = bp.BuyPrior(sd["card.weight"].shape[0],
                        sd["meta.weight"].shape[0],
                        bool(ck.get("use_meta", False)))
    model.load_state_dict(sd)
    return model.to(dev).eval(), ck


def make_pidx(env, ck, dev):
    idx = ck["card_idx"]
    return torch.tensor([0] + [idx.get(n, 1) for n in env.card_names()],
                        dtype=torch.long, device=dev)


def prior_logits_for(obs_np, pidx, prior, ck, dev):
    """The prior's 8 buy-slot logits for each observation row."""
    o = torch.from_numpy(obs_np).to(dev)
    sc = torch.tensor(SHOP_COLS, dtype=torch.long, device=dev)
    bc = torch.tensor(BOARD_COLS, dtype=torch.long, device=dev)
    sid = (o[:, sc + bgpy.SLOT_CARD_ID]
           * (o[:, sc + bgpy.SLOT_PRESENT] > 0)).long()
    bid = (o[:, bc + bgpy.SLOT_CARD_ID]
           * (o[:, bc + bgpy.SLOT_PRESENT] > 0)).long()
    offers = pidx[sid]
    board = pidx[bid]
    bmask = (bid > 0).float()
    turns = o[:, TURN].long().clamp(0, int(ck.get("max_turn", 40)))
    meta = torch.zeros((len(o), 1), dtype=torch.long, device=dev)
    meta_mask = torch.zeros((len(o), 1), dtype=torch.float32, device=dev)
    with torch.no_grad():
        logits, _ = prior(offers, board, bmask, meta, meta_mask, turns)
    return logits


def slot_name(obs, base, names):
    if obs[base + bgpy.SLOT_PRESENT] > 0:
        cid = int(obs[base + bgpy.SLOT_CARD_ID]) - 1
        if 0 <= cid < len(names):
            return names[cid]
    return None


def shop_offers(obs, names):
    return set(filter(None, (slot_name(obs, base, names)
                             for base in SHOP_COLS)))


def bought_name(obs, action, names):
    i = int(action) - BUY0
    if 0 <= i < len(SHOP_COLS):
        return slot_name(obs, SHOP_COLS[i], names)
    return None


def close_view(ep, turn, offers, bought):
    if int(turn) < 7:
        return
    for name in CARD_TARGETS:
        if name in offers:
            ep.card_views[name] += 1
            if name in bought:
                ep.card_bought[name] += 1


def parse_args():
    a = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    a.add_argument("--name", required=True, help="run folder under train/runs/oneturn")
    a.add_argument("--mode", choices=["turn", "game"], default="turn",
                   help="turn: the original first-turn diagnostic; game: whole games")
    a.add_argument("--init", required=True, help="checkpoint to start from")
    a.add_argument("--minutes", type=float, default=30.0,
                   help="training minutes, tests do not count (default 30)")
    a.add_argument("--envs", type=int, default=512, help="training envs (default 512)")
    a.add_argument("--vram-gb", type=float, required=True,
                   help="cap this process's GPU memory, required on CUDA")
    a.add_argument("--lr", type=float, default=2.5e-4)
    a.add_argument("--ent", type=float, default=0.01, help="entropy bonus")
    a.add_argument("--fight-reward", type=float, default=0.0,
                   help="game mode: +W for a fight win, -W for a loss")
    a.add_argument("--place-reward", type=float, default=0.0,
                   help="game mode: P * (4.5 - place) / 3.5 at the last action")
    a.add_argument("--gamma", type=float, default=1.0,
                   help="game mode: discount once per turn end (default 1)")
    a.add_argument("--no-moves", action="store_true",
                   help="clear move actions when another action is legal")
    a.add_argument("--deadlines", action="store_true",
                   help="game mode: add Era's levelling deadline penalty")
    a.add_argument("--lag-mix", action="store_true",
                   help="game mode: real seats draw lag 0/1/2 from turn 7")
    a.add_argument("--prior", default=None,
                   help="game mode: BuyPrior checkpoint for the aux loss")
    a.add_argument("--prior-weight", type=float, default=0.0,
                   help="game mode: weight of the BuyPrior cross-entropy")
    a.add_argument("--entropy-moves-as-one", action="store_true",
                   help="game mode: count all move actions as one entropy atom")
    a.add_argument("--seed", type=int, default=0)
    a.add_argument("--cpu", action="store_true", help="run on the CPU")
    return a.parse_args()


class Episode:
    """One env's first turn, kept until its outcome is known."""

    __slots__ = ("obs", "mask", "act", "logp", "val", "cost", "start_board",
                 "end_board", "gold", "entropy", "end_prob")

    def __init__(self):
        self.obs = []
        self.mask = []
        self.act = []
        self.logp = []
        self.val = []
        self.cost = []
        self.start_board = None
        self.end_board = None
        self.gold = None
        self.entropy = []
        self.end_prob = []


class GameEpisode:
    """One env's whole game, or the statistics of one game for a test."""

    __slots__ = ("store", "obs", "mask", "act", "logp", "val", "entropy",
                 "end_prob", "reward", "turn_end", "pending_idx", "pending_turn",
                 "place", "fights", "level_t2", "end_tier", "start_ha",
                 "turn_counts", "deadline_penalty", "prior", "buy_n", "buy_hit",
                 "buy_n7", "buy_hit7", "card_views", "card_bought")

    def __init__(self, store):
        self.store = store
        self.obs = [] if store else None
        self.mask = [] if store else None
        self.act = [] if store else None
        self.logp = [] if store else None
        self.val = [] if store else None
        self.entropy = [] if store else None
        self.end_prob = [] if store else None
        self.reward = [] if store else None
        self.turn_end = [] if store else None
        self.pending_idx = None
        self.pending_turn = 0
        self.place = 0
        self.fights = {band: [0, 0, 0] for band in FIGHT_BANDS}
        self.level_t2 = False
        self.end_tier = {}
        self.start_ha = {}
        self.turn_counts = {}
        self.deadline_penalty = 0.0
        self.prior = [] if store else None
        self.buy_n = 0
        self.buy_hit = 0
        self.buy_n7 = 0
        self.buy_hit7 = 0
        self.card_views = {name: 0 for name in CARD_TARGETS}
        self.card_bought = {name: 0 for name in CARD_TARGETS}

    def record(self, turn, action, obs=None, mask=None, logp=None, val=None,
               ent=None, endp=None, prior=None):
        turn = int(turn)
        action = int(action)
        counts = self.turn_counts.setdefault(
            turn, {k: 0 for k in LATE_METRICS})
        counts["actions"] += 1
        kind = KINDS[int(KIND_OF[action])]
        if kind in counts and kind != "actions":
            counts[kind] += 1
        if turn == 2 and kind == "level":
            self.level_t2 = True
        if self.store:
            self.obs.append(obs)
            self.mask.append(mask)
            self.act.append(action)
            self.logp.append(logp)
            self.val.append(val)
            self.entropy.append(ent)
            self.end_prob.append(endp)
            self.reward.append(-float(COST_OF[action]))
            self.turn_end.append(False)
            if prior is not None:
                self.prior.append(prior)

    def mark_turn_end(self, turn):
        self.pending_turn = int(turn)
        if self.store:
            self.pending_idx = len(self.act) - 1
            self.turn_end[self.pending_idx] = True

    def add_fight(self, outcome, fight_reward=0.0):
        """Record the fight that followed pending_turn and its reward."""
        band = fight_band(self.pending_turn)
        if outcome > 0:
            self.fights[band][0] += 1
        elif outcome < 0:
            self.fights[band][2] += 1
        else:
            self.fights[band][1] += 1
        if self.store and self.pending_idx is not None:
            self.reward[self.pending_idx] += fight_reward * outcome
            self.pending_idx = None

    def fight_totals(self):
        w = sum(x[0] for x in self.fights.values())
        t = sum(x[1] for x in self.fights.values())
        l = sum(x[2] for x in self.fights.values())
        return w, t, l


class Batch:
    """Completed episodes waiting for the next update."""

    def __init__(self):
        self.obs = []
        self.mask = []
        self.act = []
        self.logp = []
        self.val = []
        self.ret = []
        self.rewards = []
        self.lengths = []
        self.wins = 0
        self.ties = 0
        self.losses = 0
        self.fights_won = 0
        self.fights_tied = 0
        self.fights_lost = 0
        self.places = []
        self.prior = []

    def samples(self):
        return len(self.act)

    def add(self, ep: Episode, reward: float, died: bool):
        n = len(ep.act)
        costs = np.asarray(ep.cost, dtype=np.float64)
        # return at step i = reward - the move costs from i to the turn's end
        suffix = np.cumsum(costs[::-1])[::-1]
        ret = reward - suffix
        self.obs.extend(ep.obs)
        self.mask.extend(ep.mask)
        self.act.extend(ep.act)
        self.logp.extend(ep.logp)
        self.val.extend(ep.val)
        self.ret.extend(ret.tolist())
        self.rewards.append(reward)
        self.lengths.append(n)
        if reward > 0:
            self.wins += 1
        elif reward < 0:
            self.losses += 1
        else:
            self.ties += 1

    def add_game(self, ep: GameEpisode, gamma: float):
        n = len(ep.act)
        ret = np.empty(n, dtype=np.float64)
        acc = 0.0
        for t in range(n - 1, -1, -1):
            acc = ep.reward[t] + (gamma if ep.turn_end[t] else 1.0) * acc
            ret[t] = acc
        self.obs.extend(ep.obs)
        self.mask.extend(ep.mask)
        self.act.extend(ep.act)
        self.logp.extend(ep.logp)
        self.val.extend(ep.val)
        self.ret.extend(ret.tolist())
        self.rewards.append(float(sum(ep.reward)))
        self.lengths.append(n)
        if ep.prior:
            self.prior.extend(ep.prior)
        w, t, l = ep.fight_totals()
        self.fights_won += w
        self.fights_tied += t
        self.fights_lost += l
        self.places.append(ep.place)

    def clear(self):
        self.__init__()


def make_env(n: int, seed: int, split: str, real):
    return bgpy.VecEnv(n, seed, LINEUP, real_boards=real, old_boards=OLD_BOARDS,
                       real_lobby_filter=True,
                       era_starts=str(FROZEN_ACTIONS), era_share=1.0,
                       era_turn_min=ERA_TURN_MIN, era_turn_max=ERA_TURN_MAX,
                       era_split=split)


def make_game_env(n: int, seed: int, real, lag_mix=False):
    kw = dict(real_lag_mix=LAG_MIX, real_lag_from=7) if lag_mix else {}
    return bgpy.VecEnv(n, seed, LINEUP, real_boards=real, old_boards=OLD_BOARDS,
                       real_lobby_filter=True, **kw)


def throwaway(mask_row):
    if mask_row[END_TURN]:
        return END_TURN
    legal = np.nonzero(mask_row)[0]
    return int(legal[0]) if len(legal) else END_TURN


def board_total(env, e, seat):
    board = env.seats(e)[seat][5]
    return int(sum(m[2] + m[3] for m in board))


def policy_act(net, obs, mask, generator=None):
    """Sample one action from the network over the legal (forced) mask."""
    with torch.no_grad():
        logits, value = net(obs)
        dist = Categorical(logits=masked(logits, mask))
        if generator is None:
            act = dist.sample()
        else:
            act = torch.multinomial(dist.probs, 1, generator=generator).squeeze(1)
        return (act, dist.log_prob(act), value, dist.entropy(),
                dist.probs[:, END_TURN])


def outcome_after(places, e, seat, obs_next=None):
    """(reward, died) for a first turn whose ending step has happened."""
    place = int(places[e, seat])
    if place:
        return (1.0 if place == 1 else -1.0), place > 1
    if obs_next is not None and obs_next[WON] > 0.5:
        return 1.0, False
    if obs_next is not None and obs_next[LOST] > 0.5:
        return -1.0, False
    return 0.0, False


def update(batch: Batch, net, opt, dev, update_idx, cfg, state,
           prior_weight=0.0):
    """One PPO update on the completed episodes, like train/ppo.py's loss."""
    n = batch.samples()
    obs = torch.from_numpy(np.stack(batch.obs)).to(dev)
    mask = torch.from_numpy(np.stack(batch.mask)).to(dev)
    act = torch.tensor(np.asarray(batch.act), dtype=torch.long, device=dev)
    old_logp = torch.tensor(np.asarray(batch.logp), dtype=torch.float32, device=dev)
    ret = torch.tensor(np.asarray(batch.ret), dtype=torch.float32, device=dev)
    val = torch.tensor(np.asarray(batch.val), dtype=torch.float32, device=dev)
    adv = ret - val
    prior = None
    if prior_weight and batch.prior:
        if len(batch.prior) != n:
            raise SystemExit(f"prior logits {len(batch.prior)} for {n} samples")
        prior = torch.from_numpy(np.stack(batch.prior)).to(dev)

    kls, clips, ents, plains, vls, auxs = [], [], [], [], [], []
    move_idx = torch.as_tensor(MOVE_IDX, dtype=torch.long, device=dev)
    for _ in range(EPOCHS):
        perm = torch.randperm(n, device=dev)
        for i in range(0, n, MINIBATCH):
            idx = perm[i:i + MINIBATCH]
            if idx.numel() < 256:
                continue
            ad_all = adv[idx]
            ad_mean, ad_std = ad_all.mean(), ad_all.std() + 1e-8
            m = idx.numel()
            opt.zero_grad()
            kl_s = clip_s = ent_s = plain_s = vl_s = aux_s = 0.0
            for j in range(0, m, MICRO_BATCH):
                sub = idx[j:j + MICRO_BATCH]
                w = sub.numel() / m
                logits, v = net(obs[sub])
                dist = Categorical(logits=masked(logits, mask[sub]))
                lp = dist.log_prob(act[sub])
                ratio = (lp - old_logp[sub]).exp()
                ad = (adv[sub] - ad_mean) / ad_std
                pg = torch.max(-ad * ratio,
                               -ad * ratio.clamp(1 - CLIP, 1 + CLIP)).mean()
                vl = 0.5 * ((v - ret[sub]) ** 2).mean()
                plain_ent = dist.entropy().mean()
                ent = plain_ent
                if cfg.entropy_moves_as_one:
                    probs = dist.probs
                    move_p = probs[:, move_idx].sum(1)
                    nonmove = probs.clone()
                    nonmove[:, move_idx] = 0
                    ent = (-(nonmove * torch.log(nonmove.clamp_min(1e-12))).sum(1)
                           - move_p * torch.log(move_p.clamp_min(1e-12))).mean()
                aux = torch.zeros((), device=dev)
                if prior is not None:
                    legal = mask[sub][:, BUY_ACTIONS]
                    valid = legal.sum(1) >= 2
                    if valid.any():
                        target_logits = prior[sub][valid].masked_fill(
                            ~legal[valid], -1e9)
                        target = torch.softmax(target_logits, 1)
                        buy_logits = logits[valid][:, BUY_ACTIONS].masked_fill(
                            ~legal[valid], -1e9)
                        logpred = torch.log_softmax(buy_logits, 1)
                        aux = -(target * logpred).sum(1).mean()
                if update_idx < VALUE_WARMUP:
                    loss = VF * vl
                else:
                    loss = pg + VF * vl - cfg.ent * ent
                    if prior is not None:
                        loss = loss + prior_weight * aux
                (loss * w).backward()
                with torch.no_grad():
                    kl_s += w * ((ratio - 1) - (lp - old_logp[sub])).mean().item()
                    clip_s += w * ((ratio - 1).abs() > CLIP).float().mean().item()
                    ent_s += w * ent.item()
                    plain_s += w * plain_ent.item()
                    vl_s += w * vl.item()
                    aux_s += w * aux.item()
            torch.nn.utils.clip_grad_norm_(net.parameters(), GRAD_CLIP)
            opt.step()
            kls.append(kl_s)
            clips.append(clip_s)
            ents.append(ent_s)
            plains.append(plain_s)
            vls.append(vl_s)
            auxs.append(aux_s)

    stats = {
        "entropy": float(np.mean(ents)) if ents else float("nan"),
        "entropy_plain": float(np.mean(plains)) if plains else float("nan"),
        "approx_kl": float(np.mean(kls)) if kls else float("nan"),
        "clipfrac": float(np.mean(clips)) if clips else float("nan"),
        "value_loss": float(np.mean(vls)) if vls else float("nan"),
        "prior_loss": float(np.mean(auxs)) if prior is not None and auxs
        else 0.0,
    }
    if stats["approx_kl"] > 0.03:
        state["high_kl"] += 1
    else:
        state["high_kl"] = 0
    if state["high_kl"] >= 5 and not state["halved"]:
        state["lr"] *= 0.5
        for group in opt.param_groups:
            group["lr"] = state["lr"]
        state["halved"] = True
        state["high_kl"] = 0
        print(f"  approx_kl > 0.03 for 5 updates, halving lr to {state['lr']:.3g}",
              flush=True)
    return stats


def run_test(net, real, split, do_nothing, label, minutes, dev, out_path,
             rows, no_moves=False):
    """First game's first turn on every seed, two chunks of 512 envs."""
    chunk_rows = []
    for chunk in range(TEST_CHUNKS):
        env = make_env(TEST_CHUNK, TEST_SEED_BASE + chunk, split, real)
        b = ppo.Bufs(TEST_CHUNK)
        env.kinds(b.kinds)
        n = TEST_CHUNK
        done = np.zeros(n, dtype=bool)
        pending = np.zeros(n, dtype=bool)
        active = np.ones(n, dtype=bool)
        started = np.zeros(n, dtype=bool)
        ep = [Episode() for _ in range(n)]
        rec = [None] * n
        gen = torch.Generator(device=dev.type if dev.type == "cuda" else "cpu")
        gen.manual_seed(TEST_SEED_BASE + 1000 * chunk + (0 if do_nothing else 1))
        while not done.all():
            env.observe(b.obs, b.mask, b.seat, b.kind)
            # A turn that ended last step: its combat outcome is in this obs,
            # which is the learner's next turn in the same game.
            for e in np.nonzero(pending)[0]:
                e = int(e)
                reward, died = outcome_after(b.places, e, int(b.seat[e]), b.obs[e])
                rec[e] = test_row(env, e, int(b.seat[e]), ep[e], reward, died,
                                  minutes, label, split, do_nothing)
                pending[e] = False
                done[e] = True

            if done.all():
                break

            for e in np.nonzero(active & ~started)[0]:
                e = int(e)
                ep[e].start_board = board_total(env, e, int(b.seat[e]))
                started[e] = True

            acts = np.empty(n, dtype=np.int64)
            to_sample = np.nonzero(active)[0]
            ended = np.zeros(n, dtype=bool)
            if len(to_sample):
                o = torch.from_numpy(b.obs[to_sample]).to(dev)
                fmask = ppo.force_mask(o, torch.from_numpy(b.mask[to_sample]).to(dev))
                if no_moves:
                    fmask = no_moves_mask(fmask)
                if do_nothing:
                    a_np = np.array([throwaway(b.mask[e]) for e in to_sample])
                    ent_np = np.full(len(to_sample), np.nan)
                    endp_np = np.full(len(to_sample), np.nan)
                    lp_np = np.full(len(to_sample), np.nan)
                    val_np = np.full(len(to_sample), np.nan)
                else:
                    a, lp, v, ent, endp = policy_act(net, o, fmask, generator=gen)
                    a_np = a.cpu().numpy()
                    lp_np = lp.cpu().numpy()
                    val_np = v.cpu().numpy()
                    ent_np = ent.cpu().numpy()
                    endp_np = endp.cpu().numpy()
                for k, e in enumerate(to_sample):
                    e = int(e)
                    ep[e].obs.append(b.obs[e].copy())
                    ep[e].mask.append(fmask[k].cpu().numpy().copy())
                    ep[e].act.append(int(a_np[k]))
                    ep[e].logp.append(float(lp_np[k]))
                    ep[e].val.append(float(val_np[k]))
                    ep[e].cost.append(float(COST_OF[a_np[k]]))
                    ep[e].entropy.append(float(ent_np[k]))
                    ep[e].end_prob.append(float(endp_np[k]))
                    acts[e] = int(a_np[k])
                    ended[e] = (acts[e] == END_TURN
                                or b.obs[e, ACTIONS_LEFT] <= 1)
                    ep[e].gold = float(b.obs[e, GOLD])
                    if ended[e]:
                        ep[e].end_board = board_total(env, e, int(b.seat[e]))

            for e in np.nonzero(~active | done)[0]:
                acts[e] = throwaway(b.mask[e])

            seats = b.seat.copy()
            env.step(acts, b.places, b.tier_turns, b.tribes, b.reset)

            for e in np.nonzero(active)[0]:
                e = int(e)
                reward, died = outcome_after(b.places, e, int(seats[e]))
                if died or (b.places[e, int(seats[e])] != 0):
                    if ep[e].end_board is None:
                        ep[e].end_board = board_total(env, e, int(seats[e]))
                    rec[e] = test_row(env, e, int(seats[e]), ep[e], reward, died,
                                      minutes, label, split, do_nothing)
                    active[e] = False
                    done[e] = True
                elif ended[e]:
                    active[e] = False
                    pending[e] = True

        chunk_rows.extend(r for r in rec if r is not None)
    rows.extend(chunk_rows)
    print(f"  test {label}: {len(chunk_rows)} episodes", flush=True)
    write_tests(out_path, summarize_tests(rows))
    return chunk_rows


def test_row(env, e, seat, ep, reward, died, minutes, label, split, do_nothing):
    acts = np.asarray(ep.act, dtype=np.int64)
    counts = np.bincount(KIND_OF[acts], minlength=len(KINDS))
    start = ep.start_board if ep.start_board is not None else 0
    end = ep.end_board if ep.end_board is not None else 0
    gold = ep.gold if ep.gold is not None else float("nan")
    row = {
        "label": label,
        "split": split,
        "policy": "do-nothing" if do_nothing else "net",
        "minutes": minutes,
        "episodes": 1,
        "win": 1 if reward > 0 else 0,
        "tie": 1 if reward == 0 else 0,
        "loss": 1 if reward < 0 else 0,
        "died": 1 if died else 0,
        "actions": int(len(acts)),
        "gold_left": float(gold),
        "board_start": float(start),
        "board_end": float(end),
        "board_ratio": float(end / start) if start > 0 else float("nan"),
        "entropy": float(np.mean(ep.entropy)) if ep.entropy and not do_nothing
        else float("nan"),
        "end_turn_prob": float(np.mean(ep.end_prob)) if ep.end_prob and not do_nothing
        else float("nan"),
    }
    for k, name in enumerate(KINDS):
        row[f"act_{name}"] = int(counts[k])
    return row


def test_columns():
    return (["label", "split", "policy", "minutes", "episodes", "win", "tie",
             "loss", "died", "actions", "gold_left", "board_start_mean",
             "board_start_median", "board_end_mean", "board_end_median",
             "board_ratio_median", "entropy", "end_turn_prob"]
            + [f"act_{k}" for k in KINDS])


def write_tests(path: Path, rows):
    path.parent.mkdir(parents=True, exist_ok=True)
    cols = test_columns()
    with path.open("w") as f:
        f.write("\t".join(cols) + "\n")
        for row in rows:
            f.write("\t".join(f"{row[c]:.6g}" if isinstance(row[c], float) else str(row[c])
                              for c in cols) + "\n")


def nanmean(x):
    x = np.asarray(x, dtype=np.float64)
    return float(np.nanmean(x)) if np.isfinite(x).any() else float("nan")


def nanmedian(x):
    x = np.asarray(x, dtype=np.float64)
    return float(np.nanmedian(x)) if np.isfinite(x).any() else float("nan")


def summarize_tests(rows):
    """Aggregate per-episode test rows to one row per (label, split, policy)."""
    groups = {}
    order = []
    for row in rows:
        key = (row["label"], row["split"], row["policy"])
        if key not in groups:
            groups[key] = []
            order.append(key)
        groups[key].append(row)
    out = []
    for key in order:
        rs = groups[key]
        n = len(rs)
        acts = np.array([r["actions"] for r in rs], dtype=float)
        gold = np.array([r["gold_left"] for r in rs], dtype=float)
        start = np.array([r["board_start"] for r in rs], dtype=float)
        end = np.array([r["board_end"] for r in rs], dtype=float)
        ratio = np.array([r["board_ratio"] for r in rs], dtype=float)
        ratio = ratio[~np.isnan(ratio)]
        ent = np.array([r["entropy"] for r in rs], dtype=float)
        endp = np.array([r["end_turn_prob"] for r in rs], dtype=float)
        agg = {
            "label": key[0], "split": key[1], "policy": key[2],
            "minutes": rs[0]["minutes"], "episodes": n,
            "win": np.mean([r["win"] for r in rs]),
            "tie": np.mean([r["tie"] for r in rs]),
            "loss": np.mean([r["loss"] for r in rs]),
            "died": np.mean([r["died"] for r in rs]),
            "actions": float(acts.mean()),
            "gold_left": nanmean(gold),
            "board_start_mean": nanmean(start),
            "board_start_median": nanmedian(start),
            "board_end_mean": nanmean(end),
            "board_end_median": nanmedian(end),
            "board_ratio_median": float(np.median(ratio)) if len(ratio) else float("nan"),
            "entropy": nanmean(ent),
            "end_turn_prob": nanmean(endp),
        }
        for k in KINDS:
            agg[f"act_{k}"] = float(np.mean([r[f"act_{k}"] for r in rs]))
        out.append(agg)
    return out


def run_game_test(net, real, label, minutes, dev, out_path, rows,
                  no_moves=False, prior_model=None, prior_ck=None, pidx=None):
    """The first whole game on every fixed game seed, two chunks of 512."""
    chunk_rows = []
    for chunk in range(TEST_CHUNKS):
        env = make_game_env(TEST_CHUNK, TEST_SEED_BASE + chunk, real)
        b = ppo.Bufs(TEST_CHUNK)
        env.kinds(b.kinds)
        names = env.card_names()
        if prior_model is not None and pidx is None:
            pidx = make_pidx(env, prior_ck, dev)
        n = TEST_CHUNK
        done = np.zeros(n, dtype=bool)
        pending = np.zeros(n, dtype=bool)
        active = np.ones(n, dtype=bool)
        ep = [GameEpisode(False) for _ in range(n)]
        rec = [None] * n
        view_open = np.zeros(n, dtype=bool)
        view_turn = np.zeros(n, dtype=np.int64)
        view_offers = [set() for _ in range(n)]
        view_bought = [set() for _ in range(n)]
        gen = torch.Generator(device=dev.type if dev.type == "cuda" else "cpu")
        gen.manual_seed(TEST_SEED_BASE + 1000 * chunk + 2)
        while not done.all():
            env.observe(b.obs, b.mask, b.seat, b.kind)
            for e in np.nonzero(pending)[0]:
                e = int(e)
                o = b.obs[e]
                outcome = 1 if o[WON] > 0.5 else (-1 if o[LOST] > 0.5 else 0)
                ep[e].add_fight(outcome)
                pending[e] = False
            if done.all():
                break

            turns = b.obs[:, TURN].astype(np.int64)
            for e in np.nonzero(active)[0]:
                e = int(e)
                t = int(turns[e])
                if view_open[e] and int(view_turn[e]) != t:
                    close_view(ep[e], int(view_turn[e]), view_offers[e],
                               view_bought[e])
                    view_open[e] = False
                if not view_open[e]:
                    view_offers[e] = shop_offers(b.obs[e], names)
                    view_bought[e] = set()
                    view_turn[e] = t
                    view_open[e] = True
                if t in (6, 8, 10) and t not in ep[e].start_ha:
                    ep[e].start_ha[t] = float(b.obs[e, HEALTH] + b.obs[e, ARMOR])

            acts = np.empty(n, dtype=np.int64)
            ended = np.zeros(n, dtype=bool)
            to_sample = np.nonzero(active)[0]
            if len(to_sample):
                o = torch.from_numpy(b.obs[to_sample]).to(dev)
                fmask = ppo.force_mask(o, torch.from_numpy(b.mask[to_sample]).to(dev))
                if no_moves:
                    fmask = no_moves_mask(fmask)
                a, _, _, _, _ = policy_act(net, o, fmask, generator=gen)
                a_np = a.cpu().numpy()
                prior_np = None
                if prior_model is not None:
                    prior_np = prior_logits_for(
                        b.obs[to_sample], pidx, prior_model, prior_ck, dev
                    ).cpu().numpy()
                for k, e in enumerate(to_sample):
                    e = int(e)
                    action = int(a_np[k])
                    ep[e].record(turns[e], action)
                    acts[e] = action
                    ended[e] = (acts[e] == END_TURN
                                or b.obs[e, ACTIONS_LEFT] <= 1)
                    if prior_np is not None and action in BUY_ACTIONS:
                        legal_buy = fmask[k, BUY_ACTIONS].cpu().numpy()
                        if int(legal_buy.sum()) >= 2:
                            legal_idx = np.nonzero(legal_buy)[0]
                            best = int(legal_idx[
                                int(np.argmax(prior_np[k][legal_idx]))])
                            hit = int(action - BUY0 == best)
                            ep[e].buy_n += 1
                            ep[e].buy_hit += hit
                            if int(turns[e]) >= 7:
                                ep[e].buy_n7 += 1
                                ep[e].buy_hit7 += hit
                    if action in BUY_ACTIONS:
                        name = bought_name(b.obs[e], action, names)
                        if name:
                            view_bought[e].add(name)
                    if ended[e]:
                        t = int(turns[e])
                        tier = int(b.obs[e, TIER])
                        ep[e].deadline_penalty += deadline_penalty(t, tier)
                        ep[e].end_tier[t] = tier + (
                            1 if KINDS[int(KIND_OF[action])] == "level" else 0)
                    if action == ROLL_ACTION or ended[e]:
                        if view_open[e]:
                            close_view(ep[e], int(view_turn[e]), view_offers[e],
                                       view_bought[e])
                            view_open[e] = False
            for e in np.nonzero(~active | done)[0]:
                acts[e] = throwaway(b.mask[e])

            seats = b.seat.copy()
            env.step(acts, b.places, b.tier_turns, b.tribes, b.reset)

            for e in np.nonzero(active)[0]:
                e = int(e)
                place = int(b.places[e, int(seats[e])])
                t = int(turns[e])
                if place != 0:
                    if view_open[e]:
                        close_view(ep[e], int(view_turn[e]), view_offers[e],
                                   view_bought[e])
                        view_open[e] = False
                    ep[e].mark_turn_end(t)
                    ep[e].add_fight(1 if place == 1 else -1)
                    ep[e].place = place
                    rec[e] = game_test_row(ep[e], minutes, label, no_moves)
                    active[e] = False
                    done[e] = True
                elif ended[e]:
                    ep[e].mark_turn_end(t)
                    pending[e] = True

        chunk_rows.extend(r for r in rec if r is not None)
    rows.extend(chunk_rows)
    print(f"  game test {label}: {len(chunk_rows)} games", flush=True)
    write_game_tests(out_path, summarize_game_tests(rows))
    return chunk_rows


def game_test_row(ep, minutes, label, no_moves):
    w, t, l = ep.fight_totals()
    n = w + t + l
    place = int(ep.place)
    moves_all = sum(c["move"] for c in ep.turn_counts.values())
    turns_all = len(ep.turn_counts)
    moves_7 = sum(c["move"] for turn, c in ep.turn_counts.items() if turn >= 7)
    turns_7 = sum(1 for turn in ep.turn_counts if turn >= 7)
    row = {
        "label": label,
        "policy": "no-moves" if no_moves else "moves",
        "minutes": minutes,
        "episodes": 1,
        "place": float(place),
        "top4": 1 if 1 <= place <= 4 else 0,
        "eighth": 1 if place == 8 else 0,
        "fights": n,
        "fight_win": w / n if n else float("nan"),
        "fight_tie": t / n if n else float("nan"),
        "fight_loss": l / n if n else float("nan"),
        "moves_all": moves_all,
        "turn_count_all": turns_all,
        "moves_7": moves_7,
        "turn_count_7": turns_7,
        "move_per_turn_all": moves_all / turns_all if turns_all else float("nan"),
        "move_per_turn_7": moves_7 / turns_7 if turns_7 else float("nan"),
        "level_t2": 1 if ep.level_t2 else 0,
        "tier2": ep.end_tier.get(2, float("nan")),
        "tier5": ep.end_tier.get(5, float("nan")),
        "tier7": ep.end_tier.get(7, float("nan")),
        "tier8": ep.end_tier.get(8, float("nan")),
        "tier10": ep.end_tier.get(10, float("nan")),
        "deadline_penalty": float(ep.deadline_penalty),
        "prior_agree": (ep.buy_hit / ep.buy_n if ep.buy_n else float("nan")),
        "prior_n": ep.buy_n,
        "prior_agree_7": (ep.buy_hit7 / ep.buy_n7 if ep.buy_n7
                          else float("nan")),
        "prior_n_7": ep.buy_n7,
        "health6": ep.start_ha.get(6, float("nan")),
        "health8": ep.start_ha.get(8, float("nan")),
        "health10": ep.start_ha.get(10, float("nan")),
    }
    for i, name in enumerate(CARD_TARGETS):
        row[f"card{i}_views"] = int(ep.card_views[name])
        row[f"card{i}_bought"] = int(ep.card_bought[name])
    for band in FIGHT_BANDS:
        bw, bt, bl = ep.fights[band]
        bn = bw + bt + bl
        row[f"fight_n_{band}"] = bn
        row[f"fight_win_{band}"] = bw / bn if bn else float("nan")
        row[f"fight_tie_{band}"] = bt / bn if bn else float("nan")
        row[f"fight_loss_{band}"] = bl / bn if bn else float("nan")
    for t in REPORT_TURNS:
        counts = ep.turn_counts.get(t)
        row[f"t{t}_games"] = 1 if counts else 0
        for metric in LATE_METRICS:
            row[f"t{t}_{metric}"] = (float(counts[metric]) if counts
                                     else float("nan"))
    return row


def game_test_columns():
    cols = ["label", "policy", "minutes", "episodes", "place", "top4", "eighth",
            "fights", "fight_win", "fight_tie", "fight_loss"]
    for band in FIGHT_BANDS:
        cols += [f"fight_n_{band}", f"fight_win_{band}",
                 f"fight_tie_{band}", f"fight_loss_{band}"]
    cols += ["level_t2"] + [f"tier{t}" for t in TIER_TURNS]
    cols += [f"tier{t}_games" for t in TIER_TURNS]
    cols += ["deadline_penalty", "prior_agree", "prior_n", "prior_agree_7",
             "prior_n_7", "moves_all", "turn_count_all", "moves_7",
             "turn_count_7", "move_per_turn_all", "move_per_turn_7",
             "health6", "health8", "health10"]
    for i in range(len(CARD_TARGETS)):
        cols += [f"card{i}_views", f"card{i}_bought"]
    for t in REPORT_TURNS:
        cols += [f"t{t}_games"] + [f"t{t}_{m}" for m in LATE_METRICS]
    return cols


def write_game_tests(path: Path, rows):
    path.parent.mkdir(parents=True, exist_ok=True)
    cols = game_test_columns()
    with path.open("w") as f:
        f.write("\t".join(cols) + "\n")
        for row in rows:
            f.write("\t".join(f"{row[c]:.6g}" if isinstance(row[c], float)
                              else str(row[c]) for c in cols) + "\n")


def summarize_game_tests(rows):
    groups = {}
    order = []
    for row in rows:
        key = (row["label"], row["policy"])
        if key not in groups:
            groups[key] = []
            order.append(key)
        groups[key].append(row)
    out = []
    for key in order:
        rs = groups[key]
        n = len(rs)
        fights = sum(r["fights"] for r in rs)
        wins = sum(r["fight_win"] * r["fights"] for r in rs
                   if r["fights"])
        ties = sum(r["fight_tie"] * r["fights"] for r in rs
                   if r["fights"])
        losses = sum(r["fight_loss"] * r["fights"] for r in rs
                     if r["fights"])
        agg = {
            "label": key[0],
            "policy": key[1],
            "minutes": rs[0]["minutes"],
            "episodes": n,
            "place": float(np.mean([r["place"] for r in rs])),
            "top4": float(np.mean([r["top4"] for r in rs])),
            "eighth": float(np.mean([r["eighth"] for r in rs])),
            "fights": fights,
            "fight_win": wins / fights if fights else float("nan"),
            "fight_tie": ties / fights if fights else float("nan"),
            "fight_loss": losses / fights if fights else float("nan"),
            "level_t2": float(np.mean([r["level_t2"] for r in rs])),
            "deadline_penalty": float(np.mean([r["deadline_penalty"] for r in rs])),
            "health6": nanmean([r["health6"] for r in rs]),
            "health8": nanmean([r["health8"] for r in rs]),
            "health10": nanmean([r["health10"] for r in rs]),
        }
        for t in TIER_TURNS:
            vals = np.array([r[f"tier{t}"] for r in rs], dtype=np.float64)
            agg[f"tier{t}"] = nanmean(vals)
            agg[f"tier{t}_games"] = int(np.isfinite(vals).sum())
        pn = sum(r["prior_n"] for r in rs)
        ph = sum(r["prior_agree"] * r["prior_n"] for r in rs if r["prior_n"])
        pn7 = sum(r["prior_n_7"] for r in rs)
        ph7 = sum(r["prior_agree_7"] * r["prior_n_7"]
                  for r in rs if r["prior_n_7"])
        agg["prior_agree"] = ph / pn if pn else float("nan")
        agg["prior_n"] = pn
        agg["prior_agree_7"] = ph7 / pn7 if pn7 else float("nan")
        agg["prior_n_7"] = pn7
        ma = sum(r["moves_all"] for r in rs)
        ta = sum(r["turn_count_all"] for r in rs)
        m7 = sum(r["moves_7"] for r in rs)
        t7 = sum(r["turn_count_7"] for r in rs)
        agg["moves_all"] = ma
        agg["turn_count_all"] = ta
        agg["moves_7"] = m7
        agg["turn_count_7"] = t7
        agg["move_per_turn_all"] = ma / ta if ta else float("nan")
        agg["move_per_turn_7"] = m7 / t7 if t7 else float("nan")
        for i in range(len(CARD_TARGETS)):
            agg[f"card{i}_views"] = sum(r[f"card{i}_views"] for r in rs)
            agg[f"card{i}_bought"] = sum(r[f"card{i}_bought"] for r in rs)
        for band in FIGHT_BANDS:
            bn = sum(r[f"fight_n_{band}"] for r in rs)
            bw = sum(r[f"fight_win_{band}"] * r[f"fight_n_{band}"]
                     for r in rs if r[f"fight_n_{band}"])
            bt = sum(r[f"fight_tie_{band}"] * r[f"fight_n_{band}"]
                     for r in rs if r[f"fight_n_{band}"])
            bl = sum(r[f"fight_loss_{band}"] * r[f"fight_n_{band}"]
                     for r in rs if r[f"fight_n_{band}"])
            agg[f"fight_n_{band}"] = bn
            agg[f"fight_win_{band}"] = bw / bn if bn else float("nan")
            agg[f"fight_tie_{band}"] = bt / bn if bn else float("nan")
            agg[f"fight_loss_{band}"] = bl / bn if bn else float("nan")
        for t in REPORT_TURNS:
            agg[f"t{t}_games"] = sum(r[f"t{t}_games"] for r in rs)
            for metric in LATE_METRICS:
                agg[f"t{t}_{metric}"] = nanmean(
                    [r[f"t{t}_{metric}"] for r in rs
                     if r[f"t{t}_games"]])
        out.append(agg)
    return out


def write_game_report(path: Path, name, args, rows, state, n_updates,
                      n_samples, leftover_episodes, leftover_samples):
    agg = summarize_game_tests(rows)
    lines = [f"# oneturn game {name}", "",
             f"- init: `{args.init}`",
             f"- mode game, fight reward {args.fight_reward:g}, "
             f"place reward {args.place_reward:g}, gamma {args.gamma:g}, "
             f"no-moves {args.no_moves}, deadlines {args.deadlines}, "
             f"lag-mix {args.lag_mix}, prior weight {args.prior_weight:g}, "
             f"entropy-moves-as-one {args.entropy_moves_as_one}",
             f"- prior: `{args.prior or TEST_PRIOR}`",
             f"- seed {args.seed}, envs {args.envs}, training minutes "
             f"{args.minutes:g}",
             f"- updates {n_updates}, samples {n_samples}, leftover episodes "
             f"{leftover_episodes} ({leftover_samples} samples)",
             f"- final lr {state['lr']:.3g}", ""]
    lines.append("## Tests")
    lines.append("")
    lines.append("| test | min | place | top4 | 8th | fights | win | tie | loss | "
                 "level t2 | tier 2 (n) | tier 5 (n) | tier 7 (n) | tier 8 (n) | "
                 "tier 10 (n) | deadline/game | prior agree (n) | prior 7+ (n) | "
                 "win 10+ | move/turn | move/turn 7+ | health 6 | health 8 | health 10 |")
    lines.append("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for a in agg:
        lines.append(
            f"| {a['label']} | {a['minutes']:.1f} | {fmt(a['place'], 3)} | "
            f"{fmt(a['top4'])} | {fmt(a['eighth'])} | {a['fights']} | "
            f"{fmt(a['fight_win'])} | {fmt(a['fight_tie'])} | "
            f"{fmt(a['fight_loss'])} | {fmt(a['level_t2'])} | "
            + " | ".join(f"{fmt(a[f'tier{t}'], 2)} ({a[f'tier{t}_games']})"
                         for t in TIER_TURNS)
            + f" | {fmt(a['deadline_penalty'], 3)} | "
            f"{fmt(a['prior_agree'])} ({a['prior_n']}) | "
            f"{fmt(a['prior_agree_7'])} ({a['prior_n_7']}) | "
            f"{fmt(a['fight_win_10_plus'])} | "
            f"{fmt(a['move_per_turn_all'], 2)} | "
            f"{fmt(a['move_per_turn_7'], 2)} | "
            f"{fmt(a['health6'], 2)} | {fmt(a['health8'], 2)} | "
            f"{fmt(a['health10'], 2)} |")
    lines.append("")
    start = next((a for a in agg if a["label"] == "update-0"), agg[0])
    end = next((a for a in agg if a["label"] == "end"), agg[-1])
    lines.append("## Cards, turns 7 and later (update-0 vs end)")
    lines.append("")
    lines.append("| card | start views | start bought | end views | end bought |")
    lines.append("|---|---|---|---|---|")
    for i, name in enumerate(CARD_TARGETS):
        lines.append(f"| {name} | {start[f'card{i}_views']} | "
                     f"{start[f'card{i}_bought']} | {end[f'card{i}_views']} | "
                     f"{end[f'card{i}_bought']} |")
    lines.append("")
    lines.append("## Fight bands")
    lines.append("")
    lines.append("| test | turns | fights | win | tie | loss |")
    lines.append("|---|---|---|---|---|---|")
    for a in agg:
        for band in FIGHT_BANDS:
            lines.append(f"| {a['label']} | {band} | {a[f'fight_n_{band}']} | "
                         f"{fmt(a[f'fight_win_{band}'])} | "
                         f"{fmt(a[f'fight_tie_{band}'])} | "
                         f"{fmt(a[f'fight_loss_{band}'])} |")
    lines.append("")
    lines.append("## Per turn, turns 7 and later (end test)")
    lines.append("")
    lines.append("| turn | games | actions | rolls | buys | spell buys | sells | plays | moves |")
    lines.append("|---|---|---|---|---|---|---|---|---|")
    end = next((a for a in agg if a["label"] == "end"), agg[-1])
    for t in REPORT_TURNS:
        if not end[f"t{t}_games"]:
            continue
        lines.append(f"| {t} | {end[f't{t}_games']} | "
                     f"{fmt(end[f't{t}_actions'], 2)} | "
                     f"{fmt(end[f't{t}_roll'], 2)} | "
                     f"{fmt(end[f't{t}_buy'], 2)} | "
                     f"{fmt(end[f't{t}_buy_spell'], 2)} | "
                     f"{fmt(end[f't{t}_sell'], 2)} | "
                     f"{fmt(end[f't{t}_play'], 2)} | "
                     f"{fmt(end[f't{t}_move'], 2)} |")
    lines.append("")
    lines.append("Tests are two chunks of 512 fixed normal-game seeds. The test "
                 "plays the first game of each env; move actions are removed "
                 "when another action is legal if --no-moves is set.")
    path.write_text("\n".join(lines) + "\n")


def fmt(x, digits=3):
    if x is None or (isinstance(x, float) and np.isnan(x)):
        return "-"
    if isinstance(x, float) and x == int(x) and abs(x) >= 1:
        return f"{x:.0f}"
    return f"{x:.{digits}f}"


def write_report(path: Path, name, args, rows, state, n_updates, n_samples,
                 leftover_episodes, leftover_samples):
    agg = summarize_tests(rows)
    lines = [f"# oneturn {name}", "",
             f"- init: `{args.init}`",
             f"- seed {args.seed}, envs {args.envs}, training minutes {args.minutes:g}",
             f"- updates {n_updates}, samples {n_samples}, leftover episodes "
             f"{leftover_episodes} ({leftover_samples} samples)",
             f"- final lr {state['lr']:.3g}", ""]
    lines.append("## Tests")
    lines.append("")
    lines.append("| test | split | policy | min | win | tie | loss | died | actions | "
                 "sells | rolls | buys | gold | end/start | entropy | EndTurn p |")
    lines.append("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for a in agg:
        lines.append(
            f"| {a['label']} | {a['split']} | {a['policy']} | {a['minutes']:.1f} | "
            f"{fmt(a['win'])} | {fmt(a['tie'])} | {fmt(a['loss'])} | {fmt(a['died'])} | "
            f"{fmt(a['actions'], 1)} | {fmt(a['act_sell'], 2)} | {fmt(a['act_roll'], 2)} | "
            f"{fmt(a['act_buy'] + a['act_buy_spell'], 2)} | {fmt(a['gold_left'], 1)} | "
            f"{fmt(a['board_ratio_median'], 2)} | {fmt(a['entropy'], 2)} | "
            f"{fmt(a['end_turn_prob'], 2)} |")
    lines.append("")
    lines.append("## Board totals")
    lines.append("")
    lines.append("| test | start mean | start median | end mean | end median | median end/start |")
    lines.append("|---|---|---|---|---|---|")
    for a in agg:
        lines.append(f"| {a['label']} | {fmt(a['board_start_mean'], 1)} | "
                     f"{fmt(a['board_start_median'], 1)} | {fmt(a['board_end_mean'], 1)} | "
                     f"{fmt(a['board_end_median'], 1)} | {fmt(a['board_ratio_median'], 2)} |")
    lines.append("")
    lines.append("Baselines: do-nothing on the holdout and the starting network on the "
                 "train split were run once before training; the train split was run "
                 "again at the end. Tests are two chunks of 512 fixed holdout seeds.")
    path.write_text("\n".join(lines) + "\n")


def train_game(args, real, out, net, opt, dev, state, raw_ckpt, log_f,
               log_cols, tests_path, report_path, test_prior, test_prior_ck,
               train_prior, train_prior_ck):
    """Whole-game training and the normal-game test over time."""
    rows = []
    net.eval()
    run_game_test(net, real, "update-0", 0.0, dev, tests_path, rows,
                  args.no_moves, prior_model=test_prior,
                  prior_ck=test_prior_ck)
    net.train()

    env = make_game_env(args.envs, args.seed, real, lag_mix=args.lag_mix)
    b = ppo.Bufs(args.envs)
    env.kinds(b.kinds)
    train_pidx = (make_pidx(env, train_prior_ck, dev)
                  if train_prior is not None else None)
    N = args.envs
    active = np.ones(N, dtype=bool)
    pending = np.zeros(N, dtype=bool)
    ep = [GameEpisode(True) for _ in range(N)]
    batch = Batch()
    n_updates = 0
    n_samples = 0

    wall0 = time.time()
    test_wall = 0.0
    next_test = 300.0
    limit = args.minutes * 60.0

    def elapsed():
        return time.time() - wall0 - test_wall

    def do_update():
        nonlocal n_updates, n_samples
        stats = update(batch, net, opt, dev, n_updates, args, state,
                       prior_weight=(args.prior_weight
                                     if train_prior is not None else 0.0))
        episodes = len(batch.rewards)
        samples = batch.samples()
        fights = batch.fights_won + batch.fights_tied + batch.fights_lost
        n_updates += 1
        n_samples += samples
        row = {
            "update": n_updates,
            "minutes": elapsed() / 60.0,
            "episodes": episodes,
            "samples": samples,
            "mean_reward": float(np.mean(batch.rewards)),
            "win_share": batch.fights_won / fights if fights else float("nan"),
            "mean_length": samples / max(episodes, 1),
            "entropy": stats["entropy"],
            "entropy_plain": stats.get("entropy_plain", stats["entropy"]),
            "approx_kl": stats["approx_kl"],
            "clipfrac": stats["clipfrac"],
            "value_loss": stats["value_loss"],
            "prior_loss": stats.get("prior_loss", 0.0),
            "lr": state["lr"],
        }
        log_f.write("\t".join(
            f"{row[c]:.6g}" if isinstance(row[c], (int, float)) else str(row[c])
            for c in log_cols) + "\n")
        log_f.flush()
        print(f"update {n_updates} at {row['minutes']:.1f} min: "
              f"{episodes} episodes, {samples} samples, "
              f"reward {row['mean_reward']:.3f}, fight win {row['win_share']:.3f}, "
              f"len {row['mean_length']:.1f}, ent {row['entropy']:.3f}, "
              f"ent0 {row['entropy_plain']:.3f}, "
              f"kl {row['approx_kl']:.4f}, vloss {row['value_loss']:.3f}, "
              f"prior {row['prior_loss']:.3f}, "
              f"lr {row['lr']:.3g}", flush=True)
        batch.clear()

    while elapsed() < limit:
        if elapsed() >= next_test and next_test < limit:
            net.eval()
            t0 = time.time()
            run_game_test(net, real, f"t{int(next_test // 60)}",
                          elapsed() / 60.0, dev, tests_path, rows,
                          args.no_moves, prior_model=test_prior,
                          prior_ck=test_prior_ck)
            test_wall += time.time() - t0
            net.train()
            next_test += 300.0
            continue

        env.observe(b.obs, b.mask, b.seat, b.kind)
        for e in np.nonzero(pending)[0]:
            e = int(e)
            o = b.obs[e]
            outcome = 1 if o[WON] > 0.5 else (-1 if o[LOST] > 0.5 else 0)
            ep[e].add_fight(outcome, args.fight_reward)
            pending[e] = False

        turns = b.obs[:, TURN].astype(np.int64)
        tiers = b.obs[:, TIER].astype(np.int64)
        acts = np.empty(N, dtype=np.int64)
        ended = np.zeros(N, dtype=bool)
        pen = np.zeros(N, dtype=np.float64)
        to_sample = np.nonzero(active)[0]
        if len(to_sample):
            o = torch.from_numpy(b.obs[to_sample]).to(dev)
            fmask = ppo.force_mask(o, torch.from_numpy(b.mask[to_sample]).to(dev))
            if args.no_moves:
                fmask = no_moves_mask(fmask)
            a, lp, v, ent, endp = policy_act(net, o, fmask)
            a_np = a.cpu().numpy()
            lp_np = lp.cpu().numpy()
            val_np = v.cpu().numpy()
            ent_np = ent.cpu().numpy()
            endp_np = endp.cpu().numpy()
            mask_np = fmask.cpu().numpy()
            prior_np = None
            if train_prior is not None:
                prior_np = prior_logits_for(
                    b.obs[to_sample], train_pidx, train_prior,
                    train_prior_ck, dev).cpu().numpy()
            for k, e in enumerate(to_sample):
                e = int(e)
                ep[e].record(turns[e], int(a_np[k]), b.obs[e].copy(),
                             mask_np[k].copy(), float(lp_np[k]),
                             float(val_np[k]), float(ent_np[k]),
                             float(endp_np[k]),
                             prior=(prior_np[k] if prior_np is not None
                                    else None))
                acts[e] = int(a_np[k])
                ended[e] = (acts[e] == END_TURN
                            or b.obs[e, ACTIONS_LEFT] <= 1)
                if ended[e] and args.deadlines:
                    pen[e] = deadline_penalty(turns[e], tiers[e])
                    ep[e].deadline_penalty += pen[e]
        for e in np.nonzero(~active)[0]:
            acts[e] = throwaway(b.mask[e])

        seats = b.seat.copy()
        env.step(acts, b.places, b.tier_turns, b.tribes, b.reset)

        finished = []
        for e in np.nonzero(active)[0]:
            e = int(e)
            place = int(b.places[e, int(seats[e])])
            t = int(turns[e])
            if place != 0:
                if args.deadlines:
                    ep[e].reward[-1] -= pen[e]
                ep[e].mark_turn_end(t)
                ep[e].add_fight(1 if place == 1 else -1, args.fight_reward)
                ep[e].place = place
                ep[e].reward[-1] += args.place_reward * (4.5 - place) / 3.5
                finished.append(ep[e])
                ep[e] = None
                active[e] = False
            elif ended[e]:
                if args.deadlines:
                    ep[e].reward[-1] -= pen[e]
                ep[e].mark_turn_end(t)
                pending[e] = True

        for e in np.nonzero(b.reset)[0]:
            e = int(e)
            ep[e] = GameEpisode(True)
            active[e] = True
            pending[e] = False

        for done_ep in finished:
            batch.add_game(done_ep, args.gamma)
            if batch.samples() >= TARGET_SAMPLES:
                do_update()

    net.eval()
    t0 = time.time()
    run_game_test(net, real, "end", elapsed() / 60.0, dev, tests_path, rows,
                  args.no_moves, prior_model=test_prior,
                  prior_ck=test_prior_ck)
    test_wall += time.time() - t0

    leftover_episodes = int(active.sum())
    leftover_samples = int(sum(len(ep[e].act) for e in range(N)
                               if active[e] and ep[e] is not None))

    raw_ckpt["net"] = {k: v.detach().cpu() for k, v in net.state_dict().items()}
    raw_ckpt["oneturn"] = {
        "init": str(args.init), "name": args.name, "mode": "game",
        "seed": args.seed, "minutes": args.minutes, "updates": n_updates,
        "samples": n_samples, "lr": state["lr"],
        "fight_reward": args.fight_reward, "place_reward": args.place_reward,
        "gamma": args.gamma, "no_moves": bool(args.no_moves),
        "deadlines": bool(args.deadlines), "lag_mix": bool(args.lag_mix),
        "prior": str(args.prior) if args.prior else None,
        "prior_weight": args.prior_weight,
        "entropy_moves_as_one": bool(args.entropy_moves_as_one),
    }
    torch.save(raw_ckpt, out / "final.pt")
    write_game_report(report_path, args.name, args, rows, state, n_updates,
                      n_samples, leftover_episodes, leftover_samples)
    print(f"wrote {out / 'log.tsv'}, {tests_path}, {out / 'final.pt'}, "
          f"{report_path}", flush=True)


def main():
    args = parse_args()
    if args.envs < 1:
        raise SystemExit("--envs must be at least 1")
    if args.minutes <= 0:
        raise SystemExit("--minutes must be positive")

    dev = torch.device("cpu" if args.cpu else
                       ("cuda" if torch.cuda.is_available() else "cpu"))
    if dev.type == "cuda":
        total = torch.cuda.get_device_properties(0).total_memory / 2 ** 30
        torch.cuda.set_per_process_memory_fraction(min(1.0, args.vram_gb / total), 0)
        print(f"device cuda, {args.vram_gb:g} GB of {total:.1f} GB cap", flush=True)
    else:
        print("device cpu", flush=True)

    n_real = check_boards(FROZEN)
    n_actions = check_boards(FROZEN_ACTIONS)
    real, unknown = real_boards.load(FROZEN)
    if not real:
        raise SystemExit(f"{FROZEN}: no real boards")
    print(f"real boards: {n_real} files, {len(real)} boards"
          + (f", {len(set(unknown))} unmatched minion names" if unknown else ""),
          flush=True)
    print(f"Era starts: {n_actions} files, turns {ERA_TURN_MIN}..{ERA_TURN_MAX}, "
          f"split train/holdout", flush=True)

    out = OUT_ROOT / args.name
    out.mkdir(parents=True, exist_ok=True)
    log_path = out / "log.tsv"
    tests_path = out / "tests.tsv"
    report_path = out / "report.md"

    print(f"loading {args.init}", flush=True)
    info = load_ckpt(args.init, dev)
    raw_ckpt = torch.load(args.init, map_location="cpu", weights_only=False)
    ppo.FORCE_DARK = info["force_dark_gift"]
    ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = OLD_BOARDS
    ppo.REAL_LOBBY_FILTER = True
    net = info["net"]
    print(f"  update {info['update']}, force_dark_gift {info['force_dark_gift']}, "
          f"old_boards {info['old_boards']}, comp inputs {info['n_comps']}", flush=True)

    torch.manual_seed(args.seed)
    np.random.seed(args.seed)
    opt = torch.optim.Adam(net.parameters(), lr=args.lr, eps=1e-5)
    state = {"lr": args.lr, "high_kl": 0, "halved": False}

    log_f = log_path.open("w")
    log_cols = ["update", "minutes", "episodes", "samples", "mean_reward",
                "win_share", "mean_length", "entropy", "entropy_plain",
                "approx_kl", "clipfrac", "value_loss", "prior_loss", "lr"]
    log_f.write("\t".join(log_cols) + "\n")
    log_f.flush()

    if args.mode == "game":
        if args.prior_weight > 0 and not args.prior:
            raise SystemExit("--prior-weight needs --prior")
        test_prior_path = Path(args.prior or TEST_PRIOR)
        if not test_prior_path.is_file():
            raise SystemExit(f"prior {test_prior_path} does not exist")
        test_prior, test_prior_ck = load_prior(test_prior_path, dev)
        train_prior = train_prior_ck = None
        if args.prior and args.prior_weight > 0:
            train_prior_path = Path(args.prior)
            train_prior, train_prior_ck = load_prior(train_prior_path, dev)
            print(f"prior {train_prior_path} weight {args.prior_weight:g}",
                  flush=True)
        train_game(args, real, out, net, opt, dev, state, raw_ckpt, log_f,
                   log_cols, tests_path, report_path, test_prior,
                   test_prior_ck, train_prior, train_prior_ck)
        log_f.close()
        return

    rows = []
    # Baselines and the update-0 test, before training. Their time is not
    # training time.
    net.eval()
    run_test(net, real, "holdout", False, "update-0", 0.0, dev, tests_path, rows,
             args.no_moves)
    run_test(net, real, "holdout", True, "do-nothing", 0.0, dev, tests_path, rows,
             args.no_moves)
    run_test(net, real, "train", False, "train-start", 0.0, dev, tests_path, rows,
             args.no_moves)
    net.train()

    env = make_env(args.envs, args.seed, "train", real)
    b = ppo.Bufs(args.envs)
    env.kinds(b.kinds)
    N = args.envs
    active = np.ones(N, dtype=bool)
    pending = np.zeros(N, dtype=bool)
    ep = [Episode() for _ in range(N)]
    batch = Batch()
    n_updates = 0
    n_samples = 0

    wall0 = time.time()
    test_wall = 0.0
    next_test = 300.0
    limit = args.minutes * 60.0

    def elapsed():
        return time.time() - wall0 - test_wall

    while elapsed() < limit:
        if elapsed() >= next_test and next_test < limit:
            net.eval()
            t0 = time.time()
            run_test(net, real, "holdout", False, f"t{int(next_test // 60)}",
                     elapsed() / 60.0, dev, tests_path, rows, args.no_moves)
            test_wall += time.time() - t0
            net.train()
            next_test += 300.0
            continue

        env.observe(b.obs, b.mask, b.seat, b.kind)
        for e in np.nonzero(pending)[0]:
            e = int(e)
            reward, _ = outcome_after(b.places, e, int(b.seat[e]), b.obs[e])
            batch.add(ep[e], reward, False)
            ep[e] = None
            pending[e] = False

        acts = np.empty(N, dtype=np.int64)
        to_sample = np.nonzero(active)[0]
        ended = np.zeros(N, dtype=bool)
        if len(to_sample):
            o = torch.from_numpy(b.obs[to_sample]).to(dev)
            fmask = ppo.force_mask(o, torch.from_numpy(b.mask[to_sample]).to(dev))
            if args.no_moves:
                fmask = no_moves_mask(fmask)
            a, lp, v, ent, endp = policy_act(net, o, fmask)
            a_np = a.cpu().numpy()
            lp_np = lp.cpu().numpy()
            val_np = v.cpu().numpy()
            ent_np = ent.cpu().numpy()
            endp_np = endp.cpu().numpy()
            mask_np = fmask.cpu().numpy()
            for k, e in enumerate(to_sample):
                e = int(e)
                ep[e].obs.append(b.obs[e].copy())
                ep[e].mask.append(mask_np[k].copy())
                ep[e].act.append(int(a_np[k]))
                ep[e].logp.append(float(lp_np[k]))
                ep[e].val.append(float(val_np[k]))
                ep[e].cost.append(float(COST_OF[a_np[k]]))
                ep[e].entropy.append(float(ent_np[k]))
                ep[e].end_prob.append(float(endp_np[k]))
                acts[e] = int(a_np[k])
                ended[e] = (acts[e] == END_TURN
                            or b.obs[e, ACTIONS_LEFT] <= 1)
        for e in np.nonzero(~active)[0]:
            acts[e] = throwaway(b.mask[e])

        seats = b.seat.copy()
        env.step(acts, b.places, b.tier_turns, b.tribes, b.reset)

        for e in np.nonzero(active)[0]:
            e = int(e)
            place = int(b.places[e, int(seats[e])])
            if place != 0:
                reward = 1.0 if place == 1 else -1.0
                batch.add(ep[e], reward, place > 1)
                ep[e] = None
                active[e] = False
            elif ended[e]:
                active[e] = False
                pending[e] = True

        for e in np.nonzero(b.reset)[0]:
            e = int(e)
            ep[e] = Episode()
            active[e] = True
            pending[e] = False

        if batch.samples() >= TARGET_SAMPLES:
            stats = update(batch, net, opt, dev, n_updates, args, state)
            episodes = len(batch.rewards)
            samples = batch.samples()
            n_updates += 1
            n_samples += samples
            row = {
                "update": n_updates,
                "minutes": elapsed() / 60.0,
                "episodes": episodes,
                "samples": samples,
                "mean_reward": float(np.mean(batch.rewards)),
                "win_share": batch.wins / max(episodes, 1),
                "mean_length": samples / max(episodes, 1),
                "entropy": stats["entropy"],
                "entropy_plain": stats.get("entropy_plain", stats["entropy"]),
                "approx_kl": stats["approx_kl"],
                "clipfrac": stats["clipfrac"],
                "value_loss": stats["value_loss"],
                "prior_loss": stats.get("prior_loss", 0.0),
                "lr": state["lr"],
            }
            log_f.write("\t".join(
                f"{row[c]:.6g}" if isinstance(row[c], (int, float)) else str(row[c])
                for c in log_cols) + "\n")
            log_f.flush()
            print(f"update {n_updates} at {row['minutes']:.1f} min: "
                  f"{episodes} episodes, {samples} samples, "
                  f"reward {row['mean_reward']:.3f}, win {row['win_share']:.3f}, "
                  f"len {row['mean_length']:.1f}, ent {row['entropy']:.3f}, "
                  f"kl {row['approx_kl']:.4f}, vloss {row['value_loss']:.3f}, "
                  f"lr {row['lr']:.3g}", flush=True)
            batch.clear()

    log_f.close()

    # The end-of-training tests, also not training time.
    net.eval()
    t0 = time.time()
    run_test(net, real, "holdout", False, "end", elapsed() / 60.0, dev,
             tests_path, rows, args.no_moves)
    test_wall += time.time() - t0
    t0 = time.time()
    run_test(net, real, "train", False, "train-end", elapsed() / 60.0, dev,
             tests_path, rows, args.no_moves)
    test_wall += time.time() - t0

    leftover_episodes = int(active.sum())
    leftover_samples = int(sum(len(ep[e].act) for e in range(N)
                               if ep[e] is not None))

    raw_ckpt["net"] = {k: v.detach().cpu() for k, v in net.state_dict().items()}
    raw_ckpt["oneturn"] = {
        "init": str(args.init), "name": args.name, "mode": "turn",
        "seed": args.seed, "minutes": args.minutes, "updates": n_updates,
        "samples": n_samples, "lr": state["lr"],
        "no_moves": bool(args.no_moves),
    }
    torch.save(raw_ckpt, out / "final.pt")

    write_report(report_path, args.name, args, rows, state, n_updates, n_samples,
                 leftover_episodes, leftover_samples)
    print(f"wrote {log_path}, {tests_path}, {out / 'final.pt'}, {report_path}",
          flush=True)


if __name__ == "__main__":
    main()
