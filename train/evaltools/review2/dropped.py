"""Replay ppo.py's rollout bookkeeping: how many learner placement rewards arrive in a rollout
where that seat has no sample yet (ppo.py then drops the reward)?"""
import sys, os
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))); sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import Bufs, force_mask, LEARNER
from model import masked
dev = torch.device('cuda'); torch.cuda.set_per_process_memory_fraction(3.0 / (torch.cuda.get_device_properties(0).total_memory / 2**30), 0)
info = fe.load_ckpt(sys.argv[1], dev); net = info['net']; ppo.FORCE_DARK = True; ppo.FORCE_TURN2 = False
real, _ = real_boards.load(ROOT + '/boards/frozen/real-2026-11-10')
N, T, P = 512, 64, bgpy.PLAYERS
env = bgpy.VecEnv(N, 14606, [2, 2, 0, 0, 0, 4], real_boards=real, old_boards='keep', real_lobby_filter=True, real_lag_mix=[(0, 1), (1, 1), (2, 1)], real_lag_from=7)
b = Bufs(N); env.kinds(b.kinds)
kept = dropped = 0; kp = []; dp = []; learner_steps = total_steps = 0
for roll in range(40):
    last = np.full((N, P), -1)
    for t in range(T):
        env.observe(b.obs, b.mask, b.seat, b.kind)
        o = torch.from_numpy(b.obs).to(dev); m = force_mask(o, torch.from_numpy(b.mask).to(dev))
        with torch.no_grad():
            a = torch.distributions.Categorical(logits=masked(net(o, comp=None)[0], m)).sample().cpu().numpy()
        le = np.nonzero(b.kind == LEARNER)[0]; last[le, b.seat[le]] = t
        learner_steps += len(le); total_steps += N
        kinds = b.kinds.copy()
        env.step(a, b.places, b.tier_turns, b.tribes, b.reset)
        if roll >= 8:   # skip the warm-up rollouts (all games start in step at first)
            for e, s in zip(*np.nonzero(b.places)):
                if kinds[e, s] == LEARNER:
                    if last[e, s] >= 0: kept += 1; kp.append(b.places[e, s])
                    else: dropped += 1; dp.append(b.places[e, s])
        if b.reset.any():
            r = np.nonzero(b.reset)[0]; last[r] = -1; env.kinds(b.kinds)
print(f'learner placements: {kept} rewarded, {dropped} dropped ({dropped/(kept+dropped):.1%} dropped)')
print(f'mean place of rewarded {np.mean(kp):.2f}, of dropped {np.mean(dp):.2f}')
print('place distribution rewarded:', np.round(np.bincount(kp, minlength=9)[1:] / len(kp), 3))
print('place distribution dropped: ', np.round(np.bincount(dp, minlength=9)[1:] / len(dp), 3))
print(f'share of steps that are learner samples: {learner_steps/total_steps:.1%}')
