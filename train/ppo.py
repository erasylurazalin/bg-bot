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
it), standard curve bots, rush curve bots, and random bots. Reward is placement
only: +1 for 1st down to -1 for 8th, given when the seat is knocked out or the
game ends, minus a tiny cost per action (--action-cost) so it doesn't shuffle
its board for free.

Everything lands in runs/<name>/: metrics.csv (one row per eval), a
TensorBoard log, ckpt.pt (latest) and best.pt (best eval against curve bots).
"""

import argparse
import copy
import csv
import random
import time
from pathlib import Path

import numpy as np
import torch
from torch.utils.tensorboard import SummaryWriter

import bgpy
from model import MODELS, OPP_BOARDS, build, ck_model, masked, model_opts, warm_start

LEARNER, SNAPSHOT = bgpy.LEARNER, bgpy.SNAPSHOT
P = bgpy.PLAYERS
KINDS = [name for name, _ in bgpy.ACTION_KINDS]
KIND_OF = torch.tensor([k for k, (_, n) in enumerate(bgpy.ACTION_KINDS) for _ in range(n)])
END_TURN = bgpy.END_TURN
SHOP_START = bgpy.SHOP_START
ACTIONS_LEFT = bgpy.SCALAR_ACTIONS_LEFT

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
]
# Columns of bgpy's tribe counts: the ten tribes in bgcore's `Tribe::EVERY`
# order, then neutral.
TRIBE_COLS = {t: k for k, t in enumerate(TRIBE_NAMES)}
TRAIN_KEYS = [
    "entropy", "approx_kl", "clipfrac", "value_loss", "explained_var", "steps_per_s",
    "actions_per_turn", "tier_t6", "tier_t10", "health_end",
]


def args():
    a = argparse.ArgumentParser()
    a.add_argument("--name", required=True)
    a.add_argument("--minutes", type=float, default=10)
    a.add_argument("--resume")
    a.add_argument("--model", choices=list(MODELS),
                   help="network (default mlp, or --init-from's); on --resume the checkpoint's")
    a.add_argument("--opp-boards", choices=OPP_BOARDS,
                   help="attn: opponent minions as tokens (full), pooled into each opponent's "
                        "token (pooled), or pooled plus the next opponent's in full (next, the default)")
    a.add_argument("--compile", action=argparse.BooleanOptionalAction,
                   help="torch.compile the attention layers (default: on for attn)")
    a.add_argument("--init-from", help="checkpoint to take matching weights from, for a new run")
    a.add_argument("--warmup", type=int, default=0, help="updates of linear lr warmup")
    a.add_argument("--seed", type=int, default=0)
    a.add_argument("--envs", type=int, default=1024)
    a.add_argument("--steps", type=int, default=64, help="steps per env per rollout")
    a.add_argument("--lineup", default="4,2,1,1,0",
                   help="learner,snapshot,standard,rush,random seats")
    a.add_argument("--lr", type=float, default=2.5e-4)
    a.add_argument("--epochs", type=int, default=3)
    a.add_argument("--minibatch", type=int, default=8192)
    a.add_argument("--clip", type=float, default=0.2)
    a.add_argument("--ent", type=float, default=0.01)
    a.add_argument("--vf", type=float, default=0.5)
    a.add_argument("--gamma", type=float, default=0.99, help="discount per turn, not per action")
    a.add_argument("--lam", type=float, default=0.95)
    a.add_argument("--action-cost", type=float, default=0.002,
                   help="reward taken off every action, like a turn timer, so it doesn't dither")
    a.add_argument("--snapshot-every", type=int, default=250, help="updates")
    a.add_argument("--keep-every", type=float, default=30, help="minutes between weights-only copies in keep/")
    a.add_argument("--stop-flat", type=float, default=0,
                   help="stop after this many minutes without gains (0: never)")
    a.add_argument("--snapshot-pool", type=int, default=20)
    a.add_argument("--eval-every", type=int, default=20, help="updates")
    a.add_argument("--eval-games", type=int, default=512)
    return a.parse_args()


def reward(place):
    return (4.5 - place) / 3.5


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


def act(net, obs, mask, greedy=False):
    logits, value = net(obs)
    dist = torch.distributions.Categorical(logits=masked(logits, mask))
    a = logits.argmax(1) if greedy else dist.sample()
    return a, dist.log_prob(a), value, dist.entropy()


@torch.no_grad()
def evaluate(net, n_games, lineup, dev, seed):
    """Play n_games with `lineup` and return (avg_place, learner_records)."""
    n = min(n_games, 1024)
    env = bgpy.VecEnv(n, seed, lineup)
    b = Bufs(n)
    env.kinds(b.kinds)
    records, done = [], 0
    while done < n_games:
        env.observe(b.obs, b.mask, b.seat, b.kind)
        a, _, _, _ = act(net, torch.from_numpy(b.obs).to(dev), torch.from_numpy(b.mask).to(dev))
        kinds = b.kinds.copy()
        env.step(a.cpu().numpy(), b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kinds[e, s] == LEARNER:
                records.append((
                    int(b.places[e, s]),
                    b.tier_turns[e, s].copy(),
                    b.tribes[e, s].copy(),
                ))
        done += int(b.reset.sum())
        if b.reset.any():
            env.kinds(b.kinds)
    avg_place = float(np.mean([r[0] for r in records]))
    return avg_place, records


@torch.no_grad()
def evaluate_vs(net, other, n_games, dev, seed):
    """4 learner seats against 4 seats of `other`. Learners' average place."""
    n = min(n_games, 1024)
    env = bgpy.VecEnv(n, seed, [4, 4, 0, 0, 0])
    b = Bufs(n)
    env.kinds(b.kinds)
    places, done = [], 0
    while done < n_games:
        env.observe(b.obs, b.mask, b.seat, b.kind)
        o, m = torch.from_numpy(b.obs).to(dev), torch.from_numpy(b.mask).to(dev)
        a, _, _, _ = act(net, o, m)
        sn = torch.from_numpy(b.kind == SNAPSHOT).to(dev)
        if sn.any():
            a[sn] = act(other, o[sn], m[sn])[0]
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


def main():
    cfg = args()
    torch.manual_seed(cfg.seed)
    random.seed(cfg.seed)
    dev = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    ck = torch.load(cfg.resume, map_location=dev) if cfg.resume else None
    if ck is not None:
        if cfg.init_from:
            raise SystemExit("--resume and --init-from don't go together")
        saved, opts = ck_model(ck)
        given = {"model": cfg.model, **{k: getattr(cfg, k) for k in model_opts(saved)}}
        for k, v in given.items():
            if v and v != {"model": saved, **opts}[k]:
                print(f"--{k.replace('_', '-')} {v} ignored: {cfg.resume} holds "
                      f"{saved} {opts}")
        cfg.model = saved
    else:
        if cfg.init_from and not cfg.model:
            cfg.model = ck_model(torch.load(cfg.init_from, map_location="cpu"))[0]
        cfg.model = cfg.model or "mlp"
        opts = model_opts(cfg.model, opp_boards=cfg.opp_boards)
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

    N, T = cfg.envs, cfg.steps
    env = bgpy.VecEnv(N, cfg.seed, lineup)
    names = env.card_names()
    if cfg.compile is None:
        cfg.compile = cfg.model == "attn"
    net = build(cfg.model, len(names), compile=cfg.compile, **opts).to(dev)
    if cfg.init_from:
        loaded, fresh, skipped, src_opts = warm_start(net, cfg.model, cfg.init_from)
        print(f"init from {cfg.init_from} ({cfg.model} {src_opts}): {len(loaded)} parameter "
              f"tensors loaded, {len(fresh)} fresh: {', '.join(fresh) or 'none'}; "
              f"not used: {', '.join(skipped) or 'none'}")
    opt = torch.optim.Adam(net.parameters(), lr=cfg.lr, eps=1e-5)
    pool = []  # snapshot state dicts, on the CPU
    update, best, t_total = 0, 9.0, 0.0
    if ck is not None:
        net.load_state_dict(ck["net"])
        opt.load_state_dict(ck["opt"])
        for g in opt.param_groups:
            g["lr"] = cfg.lr
        pool, update, best, t_total = ck["pool"], ck["update"], ck["best"], ck.get("t_total", 0.0)
        print(f"resumed {cfg.resume} at update {update}")
    if not pool:
        pool.append(copy.deepcopy({k: v.cpu() for k, v in net.state_dict().items()}))
    snap = build(cfg.model, len(names), compile=cfg.compile, **opts).to(dev)
    kind_of = KIND_OF.to(dev)
    print(f"{cfg.model} {opts or ''}: {sum(p.numel() for p in net.parameters()) / 1e6:.2f}M "
          f"parameters on {dev}{', compiled' if cfg.compile and cfg.model == 'attn' else ''}")

    tb = SummaryWriter(out / "tb")
    csv_path = out / "metrics.csv"
    new_csv = not csv_path.exists()
    csv_file = open(csv_path, "a", newline="")
    log = csv.writer(csv_file)
    if new_csv:
        log.writerow([
            "update", "minutes", "games",
            *EVAL_KEYS,
            *TRAIN_KEYS,
            *[f"act_{k}" for k in KINDS],
            "top_buys",
        ])

    b = Bufs(N)
    env.kinds(b.kinds)
    obs = torch.zeros((T, N, bgpy.OBS_SIZE), device=dev)
    masks = torch.zeros((T, N, bgpy.N_ACTIONS), dtype=torch.bool, device=dev)
    acts = torch.zeros((T, N), dtype=torch.long, device=dev)
    logps = torch.zeros((T, N), device=dev)
    vals = torch.zeros((T, N), device=dev)
    rews = torch.zeros((T, N), device=dev)
    dones = torch.zeros((T, N), device=dev)
    learn = torch.zeros((T, N), dtype=torch.bool, device=dev)
    seats = torch.zeros((T, N), dtype=torch.long, device=dev)
    # step of each (env, seat)'s latest learner sample in this rollout, -1 for none
    last = np.full((N, P), -1, np.int64)
    games = 0
    start = time.time()
    last_keep = t_total
    history, flat_best, flat_since = [], 9.0, t_total
    stats = {}

    while True:
        snap.load_state_dict(random.choice(pool))
        last.fill(-1)
        rews.fill_(-cfg.action_cost)
        dones.zero_()
        t0 = time.time()
        with torch.no_grad():
            for t in range(T):
                env.observe(b.obs, b.mask, b.seat, b.kind)
                o = torch.from_numpy(b.obs).to(dev, non_blocking=True)
                m = torch.from_numpy(b.mask).to(dev, non_blocking=True)
                is_learner = torch.from_numpy(b.kind == LEARNER).to(dev)
                a, lp, v, _ = act(net, o, m)
                sn = ~is_learner
                if sn.any():
                    sa, _, _, _ = act(snap, o[sn], m[sn])
                    a[sn] = sa
                obs[t], masks[t], acts[t], logps[t], vals[t] = o, m, a, lp, v
                learn[t] = is_learner
                seats[t] = torch.from_numpy(b.seat).to(dev)
                le = np.nonzero(b.kind == LEARNER)[0]
                last[le, b.seat[le]] = t
                kinds = b.kinds.copy()
                env.step(a.cpu().numpy(), b.places, b.tier_turns, b.tribes, b.reset)
                for e, s in zip(*np.nonzero(b.places)):
                    if kinds[e, s] == LEARNER and last[e, s] >= 0:
                        rews[last[e, s], e] += reward(b.places[e, s])
                        dones[last[e, s], e] = 1.0
                if b.reset.any():
                    r = np.nonzero(b.reset)[0]
                    last[r] = -1
                    games += len(r)
                    env.kinds(b.kinds)

            # bootstrap from every live learner seat's current view
            all_obs = np.zeros((N, P, bgpy.OBS_SIZE), np.float32)
            live = np.zeros((N, P), bool)
            env.observe_all(all_obs, live)
            _, boot = net(torch.from_numpy(all_obs).to(dev).reshape(N * P, -1))
            next_v = (boot.reshape(N, P) * torch.from_numpy(live).to(dev)).clone()
            next_a = torch.zeros((N, P), device=dev)
            adv = torch.zeros((T, N), device=dev)
            ar = torch.arange(N, device=dev)
            # an action that ends the seat's turn moves time on by one turn
            ends = (acts == END_TURN) | (obs[:, :, ACTIONS_LEFT] <= 1)
            gammas = torch.where(ends, cfg.gamma, 1.0)
            for t in reversed(range(T)):
                s = seats[t]
                nv, na = next_v[ar, s], next_a[ar, s]
                nd = 1.0 - dones[t]
                g = gammas[t]
                delta = rews[t] + g * nv * nd - vals[t]
                at = delta + g * cfg.lam * nd * na
                adv[t] = at
                lm = learn[t]
                next_v[ar[lm], s[lm]] = vals[t][lm]
                next_a[ar[lm], s[lm]] = at[lm]
            rets = adv + vals
        t_roll = time.time() - t0

        sel = learn.reshape(-1)
        f_obs = obs.reshape(T * N, -1)[sel]
        f_mask = masks.reshape(T * N, -1)[sel]
        f_act = acts.reshape(-1)[sel]
        f_logp = logps.reshape(-1)[sel]
        f_adv = adv.reshape(-1)[sel]
        f_ret = rets.reshape(-1)[sel]
        f_val = vals.reshape(-1)[sel]
        n = f_obs.shape[0]
        if cfg.warmup:
            for g in opt.param_groups:
                g["lr"] = cfg.lr * min(1.0, (update + 1) / cfg.warmup)
        kls, clips, ents, vls = [], [], [], []
        for _ in range(cfg.epochs):
            perm = torch.randperm(n, device=dev)
            for i in range(0, n, cfg.minibatch):
                idx = perm[i:i + cfg.minibatch]
                # the learner's step count varies, so the last minibatch can be tiny:
                # its advantage std is noise, or NaN for one sample (which poisoned a run)
                if idx.numel() < 256:
                    continue
                logits, v = net(f_obs[idx])
                dist = torch.distributions.Categorical(logits=masked(logits, f_mask[idx]))
                lp = dist.log_prob(f_act[idx])
                ratio = (lp - f_logp[idx]).exp()
                ad = f_adv[idx]
                ad = (ad - ad.mean()) / (ad.std() + 1e-8)
                pg = torch.max(-ad * ratio, -ad * ratio.clamp(1 - cfg.clip, 1 + cfg.clip)).mean()
                vl = 0.5 * ((v - f_ret[idx]) ** 2).mean()
                ent = dist.entropy().mean()
                loss = pg + cfg.vf * vl - cfg.ent * ent
                opt.zero_grad()
                loss.backward()
                torch.nn.utils.clip_grad_norm_(net.parameters(), 0.5)
                opt.step()
                with torch.no_grad():
                    kls.append(((ratio - 1) - (lp - f_logp[idx])).mean().item())
                    clips.append(((ratio - 1).abs() > cfg.clip).float().mean().item())
                    ents.append(ent.item())
                    vls.append(vl.item())
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
            seed = 10_000 + update * 10
            vs_std, rec_std = evaluate(net, cfg.eval_games, [1, 0, 7, 0, 0], dev, seed)
            vs_rush, rec_rush = evaluate(net, cfg.eval_games, [1, 0, 0, 7, 0], dev, seed + 1)
            v4_std, rec_4s = evaluate(net, cfg.eval_games // 2, [4, 0, 4, 0, 0], dev, seed + 2)
            v4_rush, rec_4r = evaluate(net, cfg.eval_games // 2, [4, 0, 0, 4, 0], dev, seed + 3)
            v4_mix, rec_4m = evaluate(net, cfg.eval_games // 2, [4, 0, 2, 2, 0], dev, seed + 4)
            snap.load_state_dict(pool[0])  # the oldest snapshot kept
            vs_snap = evaluate_vs(net, snap, cfg.eval_games // 2, dev, seed + 5)
            net.train()

            rep = summarize_records(rec_std + rec_rush + rec_4s + rec_4r + rec_4m)
            eval_vals = {
                "place_vs_std": vs_std,
                "place_vs_rush": vs_rush,
                "place_4v4_std": v4_std,
                "place_4v4_rush": v4_rush,
                "place_4v4_mix": v4_mix,
                "place_vs_snapshot": vs_snap,
                **rep,
            }
            for key, val in eval_vals.items():
                tb.add_scalar(f"eval/{key}", val, update)

            log.writerow([
                update, f"{minutes:.1f}", games,
                *[f"{eval_vals[k]:.3f}" for k in EVAL_KEYS],
                *[f"{stats[k]:.4g}" for k in TRAIN_KEYS],
                *[f"{stats[f'act_{k}']:.3f}" for k in KINDS],
                top_buys,
            ])
            csv_file.flush()
            score = 0.5 * (vs_std + vs_rush)
            ck = {"net": net.state_dict(), "opt": opt.state_dict(), "pool": pool, "update": update,
                  "best": best, "t_total": minutes, "cfg": vars(cfg), "model": cfg.model,
                  "model_opts": opts}
            if score < best:
                best = score
                ck["best"] = best
                torch.save(ck, out / "best.pt")
            torch.save(ck, out / "ckpt.pt")
            if minutes - last_keep >= cfg.keep_every:
                (out / "keep").mkdir(exist_ok=True)
                torch.save({"net": net.state_dict(), "update": update, "model": cfg.model,
                            "model_opts": opts}, out / "keep" / f"net_{update}.pt")
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
            print(
                f"update {update:5d}  {minutes:5.1f} min  {games:7d} games  "
                f"vs7(std {vs_std:.2f} rush {vs_rush:.2f})  "
                f"4v4(std {v4_std:.2f} rush {v4_rush:.2f} mix {v4_mix:.2f} snap {vs_snap:.2f})  "
                f"t3/4/5 {rep['turn_t3']:.1f}/{rep['turn_t4']:.1f}/{rep['turn_t5']:.1f}  "
                f"crv(std {rep['curve_standard']:.0%} rush {rep['curve_rush']:.0%} "
                f"econ {rep['curve_econ_rush']:.0%} slow {rep['curve_slow']:.0%})  "
                "trb(" + " ".join(
                    f"{t[:2]} {rep[f'tribe_{t}']:.0%}@{rep[f'place_{t}']:.2f}"
                    for t in REPORT_GROUPS) + ")  "
                f"ent {stats['entropy']:.2f}  {stats['steps_per_s']:.0f} steps/s  buys {top_buys}",
                flush=True,
            )
        if done:
            break


if __name__ == "__main__":
    main()
