"""PPO self-play for the v0.1 mini Battlegrounds.

    .venv/bin/python train/ppo.py --name smoke --minutes 10
    .venv/bin/python train/ppo.py --name run1 --resume runs/run1/ckpt.pt

Each game seats learners (this network, trained), snapshots (older copies of
it), and rule bots. Reward is placement only: +1 for 1st down to -1 for 8th,
given when the seat is knocked out or the game ends, minus a tiny cost per
action (--action-cost) so it doesn't shuffle its board for free.

Everything lands in runs/<name>/: metrics.csv (one row per eval), a
TensorBoard log, ckpt.pt (latest) and best.pt (best eval against rule bots).
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
from model import KIND_SIZES, Net, masked

LEARNER, SNAPSHOT = bgpy.LEARNER, bgpy.SNAPSHOT
P = bgpy.PLAYERS
KINDS = ["buy", "sell", "play", "target", "choose", "move", "roll", "freeze", "level", "end"]
KIND_OF = torch.tensor(sum(([k] * n for k, n in enumerate(KIND_SIZES)), []))
END_TURN = bgpy.N_ACTIONS - 1
SHOP_START = bgpy.SCALARS  # shop slots come first after the scalars
ACTIONS_LEFT = 9  # scalar index, see bgcore/src/obs.rs


def args():
    a = argparse.ArgumentParser()
    a.add_argument("--name", required=True)
    a.add_argument("--minutes", type=float, default=10)
    a.add_argument("--resume")
    a.add_argument("--seed", type=int, default=0)
    a.add_argument("--envs", type=int, default=1024)
    a.add_argument("--steps", type=int, default=64, help="steps per env per rollout")
    a.add_argument("--lineup", default="4,2,2,0", help="learner,snapshot,rule,random seats")
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
    a.add_argument("--snapshot-every", type=int, default=10, help="updates")
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
        self.reset = np.zeros(n, bool)
        self.kinds = np.zeros((n, P), np.int64)


def act(net, obs, mask, greedy=False):
    logits, value = net(obs)
    dist = torch.distributions.Categorical(logits=masked(logits, mask))
    a = logits.argmax(1) if greedy else dist.sample()
    return a, dist.log_prob(a), value, dist.entropy()


@torch.no_grad()
def evaluate(net, n_games, lineup, dev, seed):
    """Play n_games with `lineup` and return the learners' average place."""
    n = min(n_games, 1024)
    env = bgpy.VecEnv(n, seed, lineup)
    b = Bufs(n)
    env.kinds(b.kinds)
    places, done = [], 0
    while done < n_games:
        env.observe(b.obs, b.mask, b.seat, b.kind)
        a, _, _, _ = act(net, torch.from_numpy(b.obs).to(dev), torch.from_numpy(b.mask).to(dev))
        kinds = b.kinds.copy()
        env.step(a.cpu().numpy(), b.places, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kinds[e, s] == LEARNER:
                places.append(b.places[e, s])
        done += int(b.reset.sum())
        if b.reset.any():
            env.kinds(b.kinds)
    return float(np.mean(places))


def main():
    cfg = args()
    torch.manual_seed(cfg.seed)
    random.seed(cfg.seed)
    dev = torch.device("cuda" if torch.cuda.is_available() else "cpu")
    out = Path("runs") / cfg.name
    out.mkdir(parents=True, exist_ok=True)
    lineup = [int(x) for x in cfg.lineup.split(",")]

    N, T = cfg.envs, cfg.steps
    env = bgpy.VecEnv(N, cfg.seed, lineup)
    names = env.card_names()
    net = Net(len(names)).to(dev)
    opt = torch.optim.Adam(net.parameters(), lr=cfg.lr, eps=1e-5)
    pool = []  # snapshot state dicts, on the CPU
    update, best, t_total = 0, 9.0, 0.0
    if cfg.resume:
        ck = torch.load(cfg.resume, map_location=dev)
        net.load_state_dict(ck["net"])
        opt.load_state_dict(ck["opt"])
        for g in opt.param_groups:
            g["lr"] = cfg.lr
        pool, update, best, t_total = ck["pool"], ck["update"], ck["best"], ck.get("t_total", 0.0)
        print(f"resumed {cfg.resume} at update {update}")
    if not pool:
        pool.append(copy.deepcopy({k: v.cpu() for k, v in net.state_dict().items()}))
    snap = Net(len(names)).to(dev)
    kind_of = KIND_OF.to(dev)
    print(f"{sum(p.numel() for p in net.parameters()) / 1e6:.2f}M parameters on {dev}")

    tb = SummaryWriter(out / "tb")
    csv_path = out / "metrics.csv"
    new_csv = not csv_path.exists()
    csv_file = open(csv_path, "a", newline="")
    log = csv.writer(csv_file)
    if new_csv:
        log.writerow(["update", "minutes", "games", "place_vs_rule", "place_4v4_rule", "place_vs_snapshot",
                      "entropy", "approx_kl", "clipfrac", "value_loss", "explained_var", "steps_per_s",
                      "actions_per_turn",
                      "tier_t6", "tier_t10", "health_end", *[f"act_{k}" for k in KINDS], "top_buys"])

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
                env.step(a.cpu().numpy(), b.places, b.reset)
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
        kls, clips, ents, vls = [], [], [], []
        for _ in range(cfg.epochs):
            perm = torch.randperm(n, device=dev)
            for i in range(0, n, cfg.minibatch):
                idx = perm[i:i + cfg.minibatch]
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
            turn, tier, health = f_obs[:, 4], f_obs[:, 1], f_obs[:, 3]
            t6, t10 = tier[turn == 6], tier[turn == 10]
            buys = f_act[k == 0]
            card_ids = f_obs[k == 0, SHOP_START + buys * bgpy.MINION_FEATURES + 1].long() - 1
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
            seed = 10_000 + update
            vs_rule = evaluate(net, cfg.eval_games, [1, 0, 7, 0], dev, seed)
            v4 = evaluate(net, cfg.eval_games // 2, [4, 0, 4, 0], dev, seed)
            snap.load_state_dict(pool[0])  # the oldest snapshot kept
            vs_snap = evaluate_vs(net, snap, cfg.eval_games // 2, dev, seed)
            net.train()
            tb.add_scalar("eval/place_vs_rule", vs_rule, update)
            tb.add_scalar("eval/place_4v4_rule", v4, update)
            tb.add_scalar("eval/place_vs_oldest_snapshot", vs_snap, update)
            log.writerow([update, f"{minutes:.1f}", games, f"{vs_rule:.3f}", f"{v4:.3f}", f"{vs_snap:.3f}",
                          *[f"{stats[k]:.4g}" for k in ["entropy", "approx_kl", "clipfrac", "value_loss",
                                                         "explained_var", "steps_per_s",
                                                         "actions_per_turn", "tier_t6",
                                                         "tier_t10", "health_end"]],
                          *[f"{stats[f'act_{k}']:.3f}" for k in KINDS], top_buys])
            csv_file.flush()
            ck = {"net": net.state_dict(), "opt": opt.state_dict(), "pool": pool, "update": update,
                  "best": best, "t_total": minutes, "cfg": vars(cfg)}
            if vs_rule < best:
                best = vs_rule
                ck["best"] = best
                torch.save(ck, out / "best.pt")
            torch.save(ck, out / "ckpt.pt")
            print(f"update {update:5d}  {minutes:6.1f} min  {games:8d} games  "
                  f"place vs 7 rule {vs_rule:.2f}  4v4 {v4:.2f}  vs oldest snap {vs_snap:.2f}  "
                  f"ent {stats['entropy']:.2f}  {stats['actions_per_turn']:.1f} act/turn  "
                  f"tier@6 {stats['tier_t6']:.1f}  {stats['steps_per_s']:.0f} steps/s  buys {top_buys}",
                  flush=True)
        if done:
            break


@torch.no_grad()
def evaluate_vs(net, other, n_games, dev, seed):
    """4 learner seats against 4 seats of `other`. Learners' average place."""
    n = min(n_games, 1024)
    env = bgpy.VecEnv(n, seed, [4, 4, 0, 0])
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
        env.step(a.cpu().numpy(), b.places, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kinds[e, s] == LEARNER:
                places.append(b.places[e, s])
        done += int(b.reset.sum())
        if b.reset.any():
            env.kinds(b.kinds)
    return float(np.mean(places))


if __name__ == "__main__":
    main()
