"""Gold left at End Turn by turn, learner vs 7 real at lag 0 (rewritten from the review session's endgold2.py).
Usage (from train/): ../.venv/bin/python evaltools/endgold.py CKPT [VRAM_GB]"""
import sys, os
sys.path.insert(0, os.getcwd())
import numpy as np, torch, collections
import bgpy, ppo, real_boards
from model import load, masked
from ppo import LEARNER, Bufs, force_mask
CK = sys.argv[1]; VRAM = float(sys.argv[2]) if len(sys.argv) > 2 else 10
CHUNK = 1; N = 512; SEED = 20261110
dev = torch.device('cuda'); torch.cuda.set_per_process_memory_fraction(min(1.0, VRAM / 16.3), 0)
net = load(CK, dev).eval()
cfg = torch.load(CK, map_location='cpu', weights_only=False).get('cfg', {})
ppo.FORCE_DARK = bool(cfg.get('force_dark_gift')); ppo.FORCE_TURN2 = False
old = cfg.get('old_boards') or 'keep'; ppo.OLD_BOARDS = old; ppo.REAL_LOBBY_FILTER = True
real, _ = real_boards.load('../boards/frozen/real-2026-11-10')
env = bgpy.VecEnv(N, SEED + CHUNK, [1, 0, 0, 0, 0, 7], real_boards=real, old_boards=old, real_lobby_filter=True)
b = Bufs(N); env.kinds(b.kinds); done = np.zeros(N, bool); torch.manual_seed(SEED + CHUNK)
END = bgpy.END_TURN
ends = collections.defaultdict(list); first_is_end = collections.Counter(); turns_seen = collections.Counter(); acted = collections.Counter()
with torch.no_grad():
    while not done.all():
        env.observe(b.obs, b.mask, b.seat, b.kind)
        o = torch.from_numpy(b.obs).to(dev); m = force_mask(o, torch.from_numpy(b.mask).to(dev))
        logits, _ = net(o)
        a = torch.distributions.Categorical(logits=masked(logits, m)).sample(); an = a.cpu().numpy()
        turn = b.obs[:, bgpy.SCALAR_TURN].astype(int)
        for e in np.nonzero((b.kind == LEARNER) & ~done)[0]:
            key = (int(e), int(turn[e]))
            if acted[key] == 0:
                turns_seen[min(turn[e], 14)] += 1
                if an[e] == END: first_is_end[min(turn[e], 14)] += 1
            acted[key] += 1
            if an[e] == END:
                nb = len(env.seats(int(e))[int(b.seat[e])][5])
                ends[min(int(turn[e]), 14)].append((float(b.obs[e, bgpy.SCALAR_GOLD]), nb))
        kinds = b.kinds.copy()
        env.step(an, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kinds[e, s] == LEARNER: done[e] = True
        if b.reset.any(): env.kinds(b.kinds)
print("turn | turns played | gold left at End (mean) | share ending with 3+ gold | same, full board | share of turns where End was the first action")
for t in sorted(ends):
    g = np.array([x[0] for x in ends[t]]); nb = np.array([x[1] for x in ends[t]]); full = nb == 7
    print(f"{t:>4} | {len(g):5d} | {g.mean():5.2f} | {(g>=3).mean():5.1%} | {(g[full]>=3).mean() if full.any() else float('nan'):5.1%} (n={full.sum()}) | {first_is_end[t]/max(turns_seen[t],1):5.1%}")
