"""Step 3's gate read on Era's held-out starts (plan-2026-11-13): per checkpoint, the place,
levels and rolls a turn from the start turn on, and the share of the starting board's card
copies still on the board three turns later. Same env setup as fixed_eval's Era-start mode
(era_share 1.0, the given split and turns, fixed-eval seeds, 1 learner vs 7 real). Eval only.

    python train/evaltools/school_read.py BASE.pt OTHER.pt ... --vram-gb 3
"""
import sys, os, json, argparse, math
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CWD = os.getcwd()
sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import LEARNER, Bufs, force_mask, KINDS, act

a = argparse.ArgumentParser()
a.add_argument('ckpts', nargs='+')
a.add_argument('--games', type=int, default=2048)
a.add_argument('--split', default='holdout')
a.add_argument('--turns', default='7,11')
a.add_argument('--real-lag', type=int, default=2)
a.add_argument('--after', type=int, default=3, help='turns after the start for the board-kept read')
a.add_argument('--vram-gb', type=float, required=True)
a.add_argument('--out', default=ROOT + '/train/runs/school-read')
args = a.parse_args()
args.out = os.path.join(CWD, args.out)
tmin, tmax = (int(x) for x in args.turns.split(','))
KIND_OF = ppo.KIND_OF.numpy()
LEVEL, ROLL = KINDS.index('level'), KINDS.index('roll')
dev = torch.device('cuda')
torch.cuda.set_per_process_memory_fraction(
    min(1.0, args.vram_gb / (torch.cuda.get_device_properties(0).total_memory / 2**30)), 0)
real, _ = real_boards.load(str(fe.FROZEN))
era_folder = str(fe.FROZEN_ACTIONS)


def run(path):
    info = fe.load_ckpt(path, dev); net = info['net']
    ppo.FORCE_DARK = info['force_dark_gift']; ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = info['old_boards']; ppo.REAL_LOBBY_FILTER = True
    rows = []
    for chunk in range((args.games + fe.CHUNK - 1) // fe.CHUNK):
        n = min(fe.CHUNK, args.games - chunk * fe.CHUNK)
        env = bgpy.VecEnv(n, fe.SEED_BASE + chunk, fe.LINEUP, real_boards=real,
                          old_boards=info['old_boards'], real_lobby_filter=True,
                          real_lag=args.real_lag, real_lag_from=1,
                          era_starts=era_folder, era_share=1.0, era_turn_min=tmin,
                          era_turn_max=tmax, era_split=args.split)
        names = env.card_names()
        t0 = np.asarray(env.era_turns(), dtype=np.int64)
        seats = np.asarray(env.era_seats(), dtype=np.int64)
        bodies = env.era_bodies()
        b = Bufs(n); env.kinds(b.kinds); done = np.zeros(n, bool)
        levels = np.zeros(n); rolls = np.zeros(n); turns_seen = [set() for _ in range(n)]
        kept = [None] * n
        torch.manual_seed(fe.SEED_BASE + chunk)
        while not done.all():
            env.observe(b.obs, b.mask, b.seat, b.kind)
            turns = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
            learner = (~done) & (b.kind == LEARNER)
            for e in np.nonzero(learner & (turns >= t0))[0]:
                turns_seen[e].add(int(turns[e]))
                if kept[e] is None and t0[e] and turns[e] >= t0[e] + args.after:
                    body = [names[c] for c in bodies[e][int(seats[e])] if c >= 0]
                    left = [m[0] for m in env.seats(int(e))[int(b.seat[e])][5]]
                    k = 0
                    for c in body:
                        if c in left: left.remove(c); k += 1
                    kept[e] = (k, len(body))
            o = torch.from_numpy(b.obs).to(dev)
            mask = force_mask(o, torch.from_numpy(b.mask).to(dev))
            acts, _, _, _ = act(net, o, mask)
            an = acts.cpu().numpy(); kk = KIND_OF[an]
            on = learner & (turns >= t0) & (t0 > 0)
            levels += on & (kk == LEVEL); rolls += on & (kk == ROLL)
            kinds = b.kinds.copy()
            env.step(an, b.places, b.tier_turns, b.tribes, b.reset)
            for e, s in zip(*np.nonzero(b.places)):
                if kinds[e, s] != LEARNER or done[e]: continue
                rows.append(dict(game=chunk * fe.CHUNK + int(e), start=int(t0[e]),
                                 place=int(b.places[e, s]), turns=len(turns_seen[e]),
                                 levels=int(levels[e]), rolls=int(rolls[e]), kept=kept[e]))
                done[e] = True
            if b.reset.any(): env.kinds(b.kinds)
        print(f'{os.path.basename(path)} chunk {chunk}: {len(rows)}', flush=True)
    return rows


def summary(rows):
    rows = [r for r in rows if r['start']]
    p = [r['place'] for r in rows]; m = sum(p) / len(p)
    sd = math.sqrt(sum((x - m) ** 2 for x in p) / (len(p) - 1))
    T = sum(r['turns'] for r in rows)
    k = [r['kept'] for r in rows if r['kept']]
    return dict(games=len(rows), place=m, place_hw=1.96 * sd / math.sqrt(len(p)),
                levels=sum(r['levels'] for r in rows) / T, rolls=sum(r['rolls'] for r in rows) / T,
                kept=sum(x[0] for x in k) / max(1, sum(x[1] for x in k)), kept_games=len(k))


os.makedirs(args.out, exist_ok=True)
res = {p: run(os.path.join(CWD, p)) for p in args.ckpts}
json.dump(res, open(args.out + '/rows.json', 'w'))
base = res[args.ckpts[0]]
L = [f"# School read on Era's {args.split} starts (turns {tmin} to {tmax}, lag {args.real_lag})\n",
     '| checkpoint | games | place | levels a turn | rolls a turn | board kept after '
     f'{args.after} turns | paired place vs first |', '|---|---|---|---|---|---|---|']
bg = {r['game']: r['place'] for r in base}
for p, rows in res.items():
    s = summary(rows)
    d = [r['place'] - bg[r['game']] for r in rows if r['game'] in bg and r['start']]
    md = sum(d) / len(d); sdd = math.sqrt(sum((x - md) ** 2 for x in d) / (len(d) - 1)); h = 1.96 * sdd / math.sqrt(len(d))
    L.append(f"| `{p}` | {s['games']} | {s['place']:.3f} (+-{s['place_hw']:.3f}) | {s['levels']:.3f} | "
             f"{s['rolls']:.2f} | {s['kept']:.1%} ({s['kept_games']} games) | {md:+.3f} ({md-h:+.3f} to {md+h:+.3f}) |")
open(args.out + '/report.md', 'w').write('\n'.join(L) + '\n')
print('\n'.join(L))
