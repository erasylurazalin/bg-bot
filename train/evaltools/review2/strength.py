"""Is there information about the final place that the value head misses?
At each End Turn: V, Health+armor, board total, and a low-noise strength score
(mean Health lost over 20 simulated fights against real boards of this turn)."""
import sys, os, json
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))); sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import LEARNER, Bufs, force_mask
from model import masked
END = bgpy.END_TURN
ckpt, lag = sys.argv[1], int(sys.argv[2]); games = int(sys.argv[3]) if len(sys.argv) > 3 else 1024
dev = torch.device('cuda'); torch.cuda.set_per_process_memory_fraction(3.0 / (torch.cuda.get_device_properties(0).total_memory / 2**30), 0)
info = fe.load_ckpt(ckpt, dev); net = info['net']
ppo.FORCE_DARK = info['force_dark_gift']; ppo.FORCE_TURN2 = False
real, _ = real_boards.load(ROOT + '/boards/frozen/real-2026-11-10')
rec = []; places = {}
for chunk in range(games // 512):
    n = 512
    env = bgpy.VecEnv(n, 20261110 + chunk, [1, 0, 0, 0, 0, 7], real_boards=real, old_boards='keep', real_lobby_filter=True, real_lag=lag, real_lag_from=7)
    b = Bufs(n); env.kinds(b.kinds); done = np.zeros(n, bool); torch.manual_seed(chunk)
    one = np.zeros(1, np.int32)
    while not done.all():
        env.observe(b.obs, b.mask, b.seat, b.kind)
        turns = b.obs[:, bgpy.SCALAR_TURN].astype(int)
        o = torch.from_numpy(b.obs).to(dev); m = force_mask(o, torch.from_numpy(b.mask).to(dev))
        with torch.no_grad():
            logits, value = net(o, comp=None)
            acts = torch.distributions.Categorical(logits=masked(logits, m)).sample().cpu().numpy()
        v = value.cpu().numpy()
        for e in np.nonzero(~done & (acts == END))[0]:
            sb = env.search_copy(int(e)); sb.set_fights(20, 12345 + int(turns[e])); sb.fights(one)
            st = env.seats(int(e))[int(b.seat[e])]
            rec.append((chunk * 512 + int(e), int(turns[e]), float(v[e]), float(b.obs[e, bgpy.SCALAR_HEALTH] + b.obs[e, bgpy.SCALAR_ARMOR]),
                        -one[0] / max(1, sb.n_fights), float(sum(x[2] + x[3] for x in st[5]))))
        kk = b.kinds.copy()
        env.step(acts, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kk[e, s] == LEARNER and not done[e]: places[chunk * 512 + int(e)] = int(b.places[e, s]); done[e] = True
        if b.reset.any(): env.kinds(b.kinds)
json.dump(dict(rec=rec, places=places), open(sys.argv[4], 'w')) if len(sys.argv) > 4 else None
def r2(X, y):
    X = np.column_stack([np.ones(len(y))] + list(X)); beta, *_ = np.linalg.lstsq(X, y, rcond=None)
    return 1 - ((y - X @ beta) ** 2).sum() / ((y - y.mean()) ** 2).sum()
print(f'== lag {lag}, {len(places)} games, place {np.mean(list(places.values())):.3f}')
print('share of the final reward explained at End Turn, by turn (linear fit):')
print('  turn     n   mean dmg  |  V alone | Health + strength | V + Health + strength | Health + log(stats) | strength from log(stats)')
for t in range(3, 15):
    rows = [r for r in rec if r[1] == t]
    if len(rows) < 150: continue
    y = np.array([(4.5 - places[r[0]]) / 3.5 for r in rows]); V = np.array([r[2] for r in rows]); hp = np.array([r[3] for r in rows])
    dmg = np.array([r[4] for r in rows]); ls = np.log1p(np.array([r[5] for r in rows]))
    print(f'  {t:4d} {len(rows):5d}   {dmg.mean():6.2f}   |   {r2([V], y):.2f}   |       {r2([hp, dmg], y):.2f}        |         {r2([V, hp, dmg], y):.2f}          |        {r2([hp, ls], y):.2f}         |      {r2([ls], dmg):.2f}')
