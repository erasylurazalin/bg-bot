"""Which hero powers does the bot press? Per hero: turns the power could be pressed (legal
in the mask at some step of the turn), turns it was pressed, and presses a turn, all turns.
Same games as hero_check.py (1 learner vs 7 real, sampled). Eval only."""
import sys, os, json, argparse, collections
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import LEARNER, Bufs, force_mask, KINDS
from model import masked
a = argparse.ArgumentParser()
a.add_argument('ckpt'); a.add_argument('--games', type=int, default=2048)
a.add_argument('--vram-gb', type=float, required=True)
a.add_argument('--seed-base', type=int, default=20261110)
a.add_argument('--out', required=True)
args = a.parse_args()
KIND_OF = ppo.KIND_OF.numpy()
HP = {k: np.nonzero(KIND_OF == KINDS.index(k))[0] for k in ('hero_power', 'hero_power2')}
dev = torch.device('cuda')
torch.cuda.set_per_process_memory_fraction(min(1.0, args.vram_gb / (torch.cuda.get_device_properties(0).total_memory / 2**30)), 0)
info = fe.load_ckpt(args.ckpt, dev); net = info['net']
ppo.FORCE_DARK = info['force_dark_gift']; ppo.FORCE_TURN2 = False
ppo.OLD_BOARDS = info['old_boards']; ppo.REAL_LOBBY_FILTER = True
real, _ = real_boards.load(ROOT + '/boards/frozen/real-2026-11-10')
rows = []
for chunk in range((args.games + 511) // 512):
    n = min(512, args.games - chunk * 512)
    env = bgpy.VecEnv(n, args.seed_base + chunk, [1, 0, 0, 0, 0, 7], real_boards=real,
                      old_boards=info['old_boards'], real_lobby_filter=True, real_lag=0, real_lag_from=7)
    b = Bufs(n); env.kinds(b.kinds); done = np.zeros(n, bool)
    hero = [None] * n
    # per game: turn -> [legal1, used1, legal2, used2, p_hp_when_legal_sum, legal_steps]
    per = [collections.defaultdict(lambda: [0, 0, 0, 0, 0.0, 0]) for _ in range(n)]
    torch.manual_seed(args.seed_base + chunk)
    while not done.all():
        env.observe(b.obs, b.mask, b.seat, b.kind)
        turns = b.obs[:, bgpy.SCALAR_TURN].astype(int)
        o = torch.from_numpy(b.obs).to(dev)
        mask = force_mask(o, torch.from_numpy(b.mask).to(dev))
        with torch.no_grad():
            logits, _ = net(o, comp=None)
            ml = masked(logits, mask)
            p = torch.softmax(ml, 1)
            acts = torch.distributions.Categorical(logits=ml).sample()
        acts_np = acts.cpu().numpy(); m_np = mask.cpu().numpy()
        php = p[:, HP['hero_power']].sum(1).cpu().numpy()
        for e in np.nonzero(~done)[0]:
            if hero[e] is None: hero[e] = env.seats(int(e))[int(b.seat[e])][4]
            r = per[e][int(turns[e])]
            l1 = m_np[e, HP['hero_power']].any(); l2 = m_np[e, HP['hero_power2']].any()
            if l1: r[0] = 1; r[4] += float(php[e]); r[5] += 1
            if l2: r[2] = 1
            k = KINDS[KIND_OF[acts_np[e]]]
            if k == 'hero_power': r[1] += 1
            if k == 'hero_power2': r[3] += 1
        kk = b.kinds.copy()
        env.step(acts_np, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kk[e, s] != LEARNER or done[e]: continue
            rows.append(dict(hero=hero[e], place=int(b.places[e, s]), turns={t: v for t, v in per[e].items()}))
            done[e] = True
        if b.reset.any(): env.kinds(b.kinds)
    print(f'chunk {chunk}: {len(rows)}', flush=True)
os.makedirs(os.path.dirname(args.out), exist_ok=True)
json.dump(dict(ckpt=args.ckpt, rows=rows), open(args.out, 'w'))
