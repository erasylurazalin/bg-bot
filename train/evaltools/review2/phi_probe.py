"""Can the network read a board's strength? Regress a low-noise strength score
(mean Health lost over 20 simulated fights vs real boards of the turn) from the observation.
Fine-tunes a copy of the checkpoint on the value output only; held-out games for the test."""
import sys, os, copy
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))); sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import LEARNER, Bufs, force_mask
from model import masked
END = bgpy.END_TURN
ckpt = sys.argv[1]; games = int(sys.argv[2]); out = sys.argv[3]
dev = torch.device('cuda'); torch.cuda.set_per_process_memory_fraction(3.5 / (torch.cuda.get_device_properties(0).total_memory / 2**30), 0)
info = fe.load_ckpt(ckpt, dev); net = info['net']
ppo.FORCE_DARK = True; ppo.FORCE_TURN2 = False
real, _ = real_boards.load(ROOT + '/boards/frozen/real-2026-11-10')
if not os.path.exists(out + '.npz'):
    X = []; Y = []; G = []; T = []; V = []; HP = []; LS = []
    for chunk in range(games // 512):
        n = 512
        env = bgpy.VecEnv(n, 777000 + chunk, [1, 0, 0, 0, 0, 7], real_boards=real, old_boards='keep', real_lobby_filter=True, real_lag=1 + chunk % 2, real_lag_from=7)
        b = Bufs(n); env.kinds(b.kinds); done = np.zeros(n, bool); torch.manual_seed(chunk); one = np.zeros(1, np.int32)
        while not done.all():
            env.observe(b.obs, b.mask, b.seat, b.kind)
            turns = b.obs[:, bgpy.SCALAR_TURN].astype(int)
            o = torch.from_numpy(b.obs).to(dev); m = force_mask(o, torch.from_numpy(b.mask).to(dev))
            with torch.no_grad():
                logits, value = net(o, comp=None)
                acts = torch.distributions.Categorical(logits=masked(logits, m)).sample().cpu().numpy()
            v = value.cpu().numpy()
            for e in np.nonzero(~done & (acts == END) & (turns >= 5))[0]:
                sb = env.search_copy(int(e)); sb.set_fights(20, 999 + int(turns[e])); sb.fights(one)
                st = env.seats(int(e))[int(b.seat[e])]
                X.append(b.obs[e].astype(np.float16)); Y.append(-one[0] / max(1, sb.n_fights)); G.append(chunk * 512 + int(e)); T.append(int(turns[e]))
                V.append(float(v[e])); HP.append(float(b.obs[e, bgpy.SCALAR_HEALTH] + b.obs[e, bgpy.SCALAR_ARMOR])); LS.append(float(np.log1p(sum(x[2] + x[3] for x in st[5]))))
            kk = b.kinds.copy()
            env.step(acts, b.places, b.tier_turns, b.tribes, b.reset)
            for e, s in zip(*np.nonzero(b.places)):
                if kk[e, s] == LEARNER: done[e] = True
            if b.reset.any(): env.kinds(b.kinds)
        print('chunk', chunk, len(X), flush=True)
    np.savez(out, X=np.stack(X), Y=np.array(Y, np.float32), G=np.array(G), T=np.array(T), V=np.array(V), HP=np.array(HP), LS=np.array(LS))
d = np.load(out + '.npz'); X, Y, G, T, LS = d['X'], d['Y'], d['G'], d['T'], d['LS']
test = G >= int(games * 0.85); tr = ~test
print(f'{len(Y)} End Turn states, {tr.sum()} train, {test.sum()} test; strength score mean {Y.mean():.2f} std {Y.std():.2f}')
def r2(pred, y): return 1 - ((y - pred) ** 2).sum() / ((y - y.mean()) ** 2).sum()
# baselines: turn + log stats (linear), fitted on train
def lin(cols_tr, cols_te):
    A = np.column_stack([np.ones(tr.sum())] + cols_tr); beta, *_ = np.linalg.lstsq(A, Y[tr], rcond=None)
    return np.column_stack([np.ones(test.sum())] + cols_te) @ beta
tt = np.eye(21)[np.minimum(T, 20)]
print('held-out R2, turn only: %.3f' % r2(lin([tt[tr][:, 5:]], [tt[test][:, 5:]]), Y[test]))
print('held-out R2, turn + log(stats): %.3f' % r2(lin([tt[tr][:, 5:], LS[tr]], [tt[test][:, 5:], LS[test]]), Y[test]))
m = copy.deepcopy(net).train(); opt = torch.optim.Adam(m.parameters(), lr=1e-4)
Xt = torch.from_numpy(X[tr].astype(np.float32)); Yt = torch.from_numpy(Y[tr] / 10.0)
Xe = torch.from_numpy(X[test].astype(np.float32)).to(dev); Ye = Y[test]
def evaluate():
    m.eval()
    with torch.no_grad(): p = torch.cat([m(Xe[i:i + 1024], comp=None)[1] for i in range(0, len(Xe), 1024)]).cpu().numpy() * 10.0
    m.train(); return r2(p, Ye), p
print('held-out R2 before training (value head as is, rescaled linearly): %.3f' % r2(np.polyval(np.polyfit(evaluate()[1], Ye, 1), evaluate()[1]), Ye))
for ep in range(6):
    perm = torch.randperm(len(Xt))
    for i in range(0, len(perm), 512):
        idx = perm[i:i + 512]
        loss = ((m(Xt[idx].to(dev), comp=None)[1] - Yt[idx].to(dev)) ** 2).mean()
        opt.zero_grad(); loss.backward(); torch.nn.utils.clip_grad_norm_(m.parameters(), 1.0); opt.step()
    r, p = evaluate(); print(f'epoch {ep + 1}: held-out R2 {r:.3f}', flush=True)
for t in range(6, 15):
    s = T[test] == t
    if s.sum() > 100: print(f'   turn {t}: n {s.sum()} R2 {r2(p[s], Ye[s]):.3f}  (score std {Ye[s].std():.2f})')
