"""Per turn, at the start of the learner's turn: Health+armor, tier, won-last-fight share. Independent seeds."""
import sys, os
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))); sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import LEARNER, Bufs, force_mask
from model import masked
ckpt, lag, games, seed = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
temp = float(sys.argv[5]) if len(sys.argv) > 5 else 1.0
dev = torch.device('cuda'); torch.cuda.set_per_process_memory_fraction(3.0 / (torch.cuda.get_device_properties(0).total_memory / 2**30), 0)
info = fe.load_ckpt(ckpt, dev); net = info['net']; ppo.FORCE_DARK = True; ppo.FORCE_TURN2 = False
real, _ = real_boards.load(ROOT + '/boards/frozen/real-2026-11-10')
M = 18; hp = np.zeros(M); tier = np.zeros(M); won = np.zeros(M); lost = np.zeros(M); cnt = np.zeros(M); places = []
for chunk in range(games // 512):
    n = 512
    env = bgpy.VecEnv(n, seed + chunk, [1, 0, 0, 0, 0, 7], real_boards=real, old_boards='keep', real_lobby_filter=True, real_lag=lag, real_lag_from=7)
    b = Bufs(n); env.kinds(b.kinds); done = np.zeros(n, bool); last = np.zeros(n, int); torch.manual_seed(seed + chunk)
    while not done.all():
        env.observe(b.obs, b.mask, b.seat, b.kind)
        t = np.minimum(b.obs[:, bgpy.SCALAR_TURN].astype(int), M - 1)
        new = ~done & (t != last); last[new] = t[new]
        np.add.at(cnt, t[new], 1); np.add.at(hp, t[new], b.obs[new, bgpy.SCALAR_HEALTH] + b.obs[new, bgpy.SCALAR_ARMOR])
        np.add.at(tier, t[new], b.obs[new, bgpy.SCALAR_TIER]); np.add.at(won, t[new], b.obs[new, bgpy.SCALAR_WON_LAST_COMBAT]); np.add.at(lost, t[new], b.obs[new, bgpy.SCALAR_LOST_LAST_COMBAT])
        o = torch.from_numpy(b.obs).to(dev); m = force_mask(o, torch.from_numpy(b.mask).to(dev))
        with torch.no_grad():
            ml = masked(net(o, comp=None)[0], m)
            acts = (torch.distributions.Categorical(logits=ml / temp).sample() if temp > 0 else ml.argmax(1)).cpu().numpy()
        kk = b.kinds.copy()
        env.step(acts, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kk[e, s] == LEARNER and not done[e]: places.append(int(b.places[e, s])); done[e] = True
        if b.reset.any(): env.kinds(b.kinds)
p = np.array(places)
print(f'== {os.path.basename(os.path.dirname(ckpt))} lag {lag} temp {temp} seeds {seed}: place {p.mean():.3f} (+-{1.96*p.std(ddof=1)/np.sqrt(len(p)):.3f}) top4 {(p<=4).mean():.1%} firsts {(p==1).sum()} 8th {(p==8).mean():.1%}')
print('   turn: games, Health+armor at start, tier at start, won / lost the fight before')
for t in range(2, 14):
    if cnt[t] > 50: print(f'   {t:3d}: {int(cnt[t]):5d}  {hp[t]/cnt[t]:5.1f}  {tier[t]/cnt[t]:4.2f}   {won[t]/cnt[t]:.2f} / {lost[t]/cnt[t]:.2f}')
