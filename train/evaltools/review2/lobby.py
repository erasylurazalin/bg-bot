"""Training-lobby dynamics: lineup 2,2,0,0,0,4 with one network on all four network seats.
Where do network seats place, and how much of a learner's place is decided among the network seats?"""
import sys, os, json
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))); sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import Bufs, force_mask
from model import masked
ckpt, lag = sys.argv[1], int(sys.argv[2]); games = int(sys.argv[3]) if len(sys.argv) > 3 else 1024
dev = torch.device('cuda'); torch.cuda.set_per_process_memory_fraction(3.0 / (torch.cuda.get_device_properties(0).total_memory / 2**30), 0)
info = fe.load_ckpt(ckpt, dev); net = info['net']
ppo.FORCE_DARK = info['force_dark_gift']; ppo.FORCE_TURN2 = False
real, _ = real_boards.load(ROOT + '/boards/frozen/real-2026-11-10')
P = bgpy.PLAYERS; allp = []; kindsl = []
for chunk in range(games // 512):
    n = 512
    env = bgpy.VecEnv(n, 20261110 + chunk, [2, 2, 0, 0, 0, 4], real_boards=real, old_boards='keep', real_lobby_filter=True, real_lag=lag, real_lag_from=7)
    b = Bufs(n); env.kinds(b.kinds); k0 = b.kinds.copy()
    place = np.zeros((n, P), int); done = np.zeros(n, bool)
    torch.manual_seed(chunk)
    while not done.all():
        env.observe(b.obs, b.mask, b.seat, b.kind)
        o = torch.from_numpy(b.obs).to(dev); m = force_mask(o, torch.from_numpy(b.mask).to(dev))
        with torch.no_grad():
            logits, _ = net(o, comp=None)
            acts = torch.distributions.Categorical(logits=masked(logits, m)).sample().cpu().numpy()
        env.step(acts, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if not done[e]: place[e, s] = b.places[e, s]
        done |= b.reset
    allp.append(place); kindsl.append(k0)
place = np.concatenate(allp); kinds = np.concatenate(kindsl)
net_seat = (kinds == bgpy.LEARNER) | (kinds == bgpy.SNAPSHOT); real_seat = kinds == bgpy.REAL
np_ = place[net_seat].reshape(len(place), -1); rp = place[real_seat].reshape(len(place), -1)
print(f'lag {lag}: games {len(place)}; missing places {(place==0).sum()}')
print('  network seats: mean place %.2f, top4 %.1f%%, firsts %.1f%%' % (np_.mean(), (np_ <= 4).mean() * 100, (np_ == 1).mean() * 100))
print('  real seats:    mean place %.2f, top4 %.1f%%, firsts %.1f%%' % (rp.mean(), (rp <= 4).mean() * 100, (rp == 1).mean() * 100))
print('  place distribution of a network seat:', np.round(np.bincount(np_.ravel(), minlength=9)[1:] / np_.size, 3))
# how many real seats does a network seat outlast, and how many network seats
out_real = (np_[:, :, None] < rp[:, None, :]).sum(2); out_net = (np_[:, :, None] < np_[:, None, :]).sum(2)
print('  a network seat outlasts %.2f of 4 real seats and %.2f of 3 other network seats' % (out_real.mean(), out_net.mean()))
print('  share of games where all 4 network seats finish 5th to 8th: %.1f%%' % ((np_.min(1) >= 5).mean() * 100))
# variance of the reward explained by rank among network seats
r = (4.5 - np_) / 3.5; rank = out_net
print('  reward variance: total %.3f; explained by rank among network seats: %.0f%%' % (r.var(), 100 * np.corrcoef(r.ravel(), rank.ravel())[0, 1] ** 2))
