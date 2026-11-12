"""What the value head believes: change in V per action kind, V's reaction to more Gold / bigger boards,
and how noisy the return is around V. Eval only."""
import sys, os, json
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))); sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import LEARNER, Bufs, force_mask, KINDS
from model import masked
KIND_OF = ppo.KIND_OF.numpy(); END = bgpy.END_TURN
ckpt, lag = sys.argv[1], int(sys.argv[2]); games = int(sys.argv[3]) if len(sys.argv) > 3 else 1024
dev = torch.device('cuda'); torch.cuda.set_per_process_memory_fraction(3.0 / (torch.cuda.get_device_properties(0).total_memory / 2**30), 0)
info = fe.load_ckpt(ckpt, dev); net = info['net']
ppo.FORCE_DARK = info['force_dark_gift']; ppo.FORCE_TURN2 = False
real, _ = real_boards.load(ROOT + '/boards/frozen/real-2026-11-10')
rng = np.random.default_rng(0)
dv = {}      # (kind) -> list of deltas within a turn, turns 7-12
dv_end = []  # V(next turn start) - V(at End Turn), turns 7-12
start_v = [] # (game, turn, V at turn start)
keep_obs = []; places = {}
for chunk in range(games // 512):
    n = 512
    env = bgpy.VecEnv(n, 20261110 + chunk, [1, 0, 0, 0, 0, 7], real_boards=real, old_boards='keep', real_lobby_filter=True, real_lag=lag, real_lag_from=7)
    b = Bufs(n); env.kinds(b.kinds); done = np.zeros(n, bool)
    prev_v = np.full(n, np.nan); prev_kind = np.full(n, -1); prev_turn = np.zeros(n, int); prev_end = np.zeros(n, bool)
    torch.manual_seed(chunk)
    while not done.all():
        env.observe(b.obs, b.mask, b.seat, b.kind)
        turns = b.obs[:, bgpy.SCALAR_TURN].astype(int)
        o = torch.from_numpy(b.obs).to(dev); m = force_mask(o, torch.from_numpy(b.mask).to(dev))
        with torch.no_grad():
            logits, value = net(o, comp=None)
            acts = torch.distributions.Categorical(logits=masked(logits, m)).sample().cpu().numpy()
        v = value.cpu().numpy()
        for e in np.nonzero(~done)[0]:
            t = turns[e]
            if prev_kind[e] >= 0 and 7 <= prev_turn[e] <= 12:
                if prev_end[e]: dv_end.append(v[e] - prev_v[e])
                elif t == prev_turn[e]: dv.setdefault(KINDS[prev_kind[e]], []).append(v[e] - prev_v[e])
            if t != prev_turn[e] or prev_kind[e] < 0: start_v.append((chunk * 512 + int(e), int(t), float(v[e])))
            if 7 <= t <= 12 and rng.random() < 0.03: keep_obs.append(b.obs[e].copy())
        prev_v = v.copy(); prev_kind = KIND_OF[acts].copy(); prev_turn = turns.copy()
        prev_end = (acts == END) | (b.obs[:, bgpy.SCALAR_ACTIONS_LEFT] <= 1)
        kk = b.kinds.copy()
        env.step(acts, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kk[e, s] == LEARNER and not done[e]:
                places[chunk * 512 + int(e)] = int(b.places[e, s]); done[e] = True
                if 7 <= prev_turn[e] <= 12: dv_end.append((4.5 - b.places[e, s]) / 3.5 - prev_v[e])
        prev_kind[done] = -1
        if b.reset.any(): env.kinds(b.kinds)
print(f'== {os.path.basename(os.path.dirname(ckpt))} lag {lag}, {len(places)} games, place {np.mean(list(places.values())):.3f}')
print('change in V after one action inside a turn (turns 7 to 12): kind, count, mean, std, share positive')
for k, x in sorted(dv.items(), key=lambda kv: -len(kv[1])):
    x = np.array(x)
    if len(x) >= 200: print(f'   {k:13s} {len(x):7d}  mean {x.mean():+.4f}  std {x.std():.4f}  positive {np.mean(x>0):.0%}')
x = np.array(dv_end); print(f'   across the fight (End Turn to next turn start or final reward): n {len(x)} mean {x.mean():+.4f} std {x.std():.4f}')
allin = np.concatenate([np.array(v) for v in dv.values()]); print(f'   all in-turn actions: mean |dV| {np.abs(allin).mean():.4f}, std {allin.std():.4f}')
R = {g: (4.5 - p) / 3.5 for g, p in places.items()}
print('return noise around V at the start of a turn: turn, n, std of reward, std of (reward - V), explained')
for t in range(1, 15):
    rows = [(R[g], v) for g, tt, v in start_v if tt == t and g in R]
    if len(rows) < 100: continue
    r = np.array([a for a, _ in rows]); vv = np.array([b_ for _, b_ in rows])
    print(f'   {t:3d} {len(rows):6d}  {r.std():.3f}  {(r - vv).std():.3f}  {1 - (r - vv).var() / r.var():.2f}')
# perturbations
ob = torch.from_numpy(np.stack(keep_obs)).to(dev)
def V(o):
    with torch.no_grad(): return torch.cat([net(o[i:i + 1024], comp=None)[1] for i in range(0, len(o), 1024)]).cpu().numpy()
base = V(ob); print(f'perturbations on {len(ob)} states from turns 7 to 12 (mean change in V; one place = 0.286):')
def pert(name, f):
    o2 = ob.clone(); f(o2); d = V(o2) - base; print(f'   {name:38s} mean {d.mean():+.4f}  std {d.std():.4f}  positive {np.mean(d>0):.0%}')
G = bgpy.SCALAR_GOLD
pert('Gold +1', lambda o: o.__setitem__((slice(None), G), o[:, G] + 1))
pert('Gold +3', lambda o: o.__setitem__((slice(None), G), o[:, G] + 3))
pert('Gold +8', lambda o: o.__setitem__((slice(None), G), o[:, G] + 8))
pert('Gold set to 0', lambda o: o.__setitem__((slice(None), G), 0))
pert('Health +10', lambda o: o.__setitem__((slice(None), bgpy.SCALAR_HEALTH), o[:, bgpy.SCALAR_HEALTH] + 10))
SF = bgpy.SLOT_FEATURES; BS = bgpy.BOARD_START
cols = [BS + i * SF + c for i in range(bgpy.BOARD) for c in (bgpy.SLOT_ATTACK, bgpy.SLOT_HEALTH, bgpy.SLOT_FIGHT_ATTACK, bgpy.SLOT_FIGHT_HEALTH)]
for mult in (0.5, 1.5, 2.0, 4.0):
    pert(f'own board stats x{mult}', lambda o, mult=mult: o.__setitem__((slice(None), cols), o[:, cols] * mult))
