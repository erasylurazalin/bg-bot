"""H1: does the bot adapt to its hero? Per hero, place and the learner's hero power uses,
sells, rolls and buys a turn from turn 7 on, for the bot (fixed-seed games, 1 learner vs 7
real, the same loop as review2/rv.py) and for Era (boards/frozen/actions-2026-11-10).
Eval only. Writes train/runs/hero-check/{report.md,rows.json}."""
import sys, os, json, argparse, glob, collections
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import LEARNER, Bufs, force_mask, KINDS
from model import masked

a = argparse.ArgumentParser()
a.add_argument('ckpt'); a.add_argument('--games', type=int, default=2048)
a.add_argument('--lag', type=int, default=0); a.add_argument('--lag-from', type=int, default=7)
a.add_argument('--vram-gb', type=float, required=True)
a.add_argument('--seed-base', type=int, default=20261110)
a.add_argument('--from-turn', type=int, default=7)
a.add_argument('--out', default=ROOT + '/train/runs/hero-check')
args = a.parse_args()
KIND_OF = ppo.KIND_OF.numpy(); END = bgpy.END_TURN
KI = {k: i for i, k in enumerate(KINDS)}
COUNT = {'hero_power': ['hero_power'], 'sell': ['sell'], 'roll': ['roll'], 'buy': ['buy', 'buy_spell']}

dev = torch.device('cuda')
tot_gb = torch.cuda.get_device_properties(0).total_memory / 2**30
torch.cuda.set_per_process_memory_fraction(min(1.0, args.vram_gb / tot_gb), 0)
info = fe.load_ckpt(args.ckpt, dev); net = info['net']
ppo.FORCE_DARK = info['force_dark_gift']; ppo.FORCE_TURN2 = False
ppo.OLD_BOARDS = info['old_boards']; ppo.REAL_LOBBY_FILTER = True
real, _ = real_boards.load(ROOT + '/boards/frozen/real-2026-11-10')

bot = []
for chunk in range((args.games + 511) // 512):
    n = min(512, args.games - chunk * 512)
    env = bgpy.VecEnv(n, args.seed_base + chunk, [1, 0, 0, 0, 0, 7], real_boards=real,
                      old_boards=info['old_boards'], real_lobby_filter=True,
                      real_lag=args.lag, real_lag_from=args.lag_from)
    b = Bufs(n); env.kinds(b.kinds); done = np.zeros(n, bool)
    hero = [None] * n
    cnt = [collections.Counter() for _ in range(n)]
    late_turns = [set() for _ in range(n)]
    torch.manual_seed(args.seed_base + chunk)
    while not done.all():
        env.observe(b.obs, b.mask, b.seat, b.kind)
        turns = b.obs[:, bgpy.SCALAR_TURN].astype(int)
        o = torch.from_numpy(b.obs).to(dev)
        mask = force_mask(o, torch.from_numpy(b.mask).to(dev))
        with torch.no_grad():
            logits, _ = net(o, comp=None)
            acts = torch.distributions.Categorical(logits=masked(logits, mask)).sample()
        acts_np = acts.cpu().numpy()
        for e in np.nonzero(~done)[0]:
            if hero[e] is None:
                hero[e] = env.seats(int(e))[int(b.seat[e])][4]
            if turns[e] >= args.from_turn:
                late_turns[e].add(int(turns[e]))
                cnt[e][KINDS[KIND_OF[acts_np[e]]]] += 1
        kk = b.kinds.copy()
        env.step(acts_np, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kk[e, s] != LEARNER or done[e]: continue
            c = cnt[e]
            bot.append(dict(game=chunk * 512 + int(e), hero=hero[e], place=int(b.places[e, s]),
                            late_turns=len(late_turns[e]),
                            **{k: sum(c[x] for x in v) for k, v in COUNT.items()}))
            done[e] = True
        if b.reset.any(): env.kinds(b.kinds)
    print(f'chunk {chunk}: {len(bot)} games', flush=True)

era = []
for f in sorted(glob.glob(ROOT + '/boards/frozen/actions-2026-11-10/*.json')):
    d = json.load(open(f))
    rb = ROOT + f"/boards/real/{d['source']}-g{d['game']}.json"
    if not os.path.exists(rb): continue
    place = json.load(open(rb))['place']
    c = collections.Counter(); late = set()
    for t in d['turns']:
        if t['turn'] < args.from_turn: continue
        late.add(t['turn'])
        for x in t['actions']: c[x['step']] += 1
    era.append(dict(game=f"{d['source']}-g{d['game']}", hero=d['hero']['name'], place=place,
                    late_turns=len(late), **{k: sum(c[x] for x in v) for k, v in COUNT.items()}))

def summ(rows):
    g = len(rows); lt = sum(r['late_turns'] for r in rows)
    out = dict(games=g, place=sum(r['place'] for r in rows) / g, late_turns=lt)
    for k in COUNT: out[k] = sum(r[k] for r in rows) / max(1, lt)
    return out

os.makedirs(args.out, exist_ok=True)
json.dump(dict(ckpt=args.ckpt, lag=args.lag, bot=bot, era=era), open(args.out + '/rows.json', 'w'))
by = lambda rows: {h: summ([r for r in rows if r['hero'] == h]) for h in {r['hero'] for r in rows}}
B, E = by(bot), by(era); ab, ae = summ(bot), summ(era)
f = lambda s: f"{s['games']} | {s['place']:.2f} | {s['hero_power']:.2f} | {s['sell']:.2f} | {s['roll']:.2f} | {s['buy']:.2f}"
L = [f"# H1: per-hero play, bot vs Era\n",
     f"- bot: `{args.ckpt}`, lag {args.lag}, {len(bot)} games, seeds {args.seed_base}+chunk, 1 learner vs 7 real, sampled at temp 1",
     f"- Era: {len(era)} games from boards/frozen/actions-2026-11-10 (place from boards/real)",
     f"- actions are per turn played from turn {args.from_turn} on; buys include Tavern spells; hero power is active uses only (passive powers show 0)\n",
     "| who | games | place | hero power | sells | rolls | buys |", "|---|---|---|---|---|---|---|",
     f"| bot, all | {f(ab)} |", f"| Era, all | {f(ae)} |\n",
     "## Heroes Era played (Era's games 1 or more)\n",
     "| hero | bot games | bot place | bot HP | bot sells | bot rolls | bot buys | Era games | Era place | Era HP | Era sells | Era rolls | Era buys |",
     "|---|" + "---|" * 12]
for h in sorted(E, key=lambda h: -E[h]['games']):
    bs = f(B[h]) if h in B else "not drawn | | | | |"
    L.append(f"| {h} | {bs} | {f(E[h])} |")
L += ["\n## Bot heroes, 20 or more games, by place\n",
      "| hero | games | place | hero power | sells | rolls | buys |", "|---|---|---|---|---|---|---|"]
for h in sorted((h for h in B if B[h]['games'] >= 20), key=lambda h: B[h]['place']):
    L.append(f"| {h} | {f(B[h])} |")
open(args.out + '/report.md', 'w').write('\n'.join(L) + '\n')
print('\n'.join(L[:12]))
