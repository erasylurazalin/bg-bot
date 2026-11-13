"""T1 (plan-2026-11-13, side step T): forced hero power press. Same seeds and loop as
train/fixed_eval.py (chunks of 512, VecEnv(n, seed_base + chunk), lineup 1 learner + 7 real,
lag 0, lobby filter, sampled through ppo.act at temp 1). Arms:
  none:  normal play;
  early: whenever a hero power action is legal and no hero power was pressed this turn, the
         mask is cut to the hero power actions (the network still picks among them and
         picks any target on the next step);
  late:  the same cut, but only at a step where the network picked End Turn.
Writes rows (game, hero, place, forced presses, presses) to --out. Eval only."""
import sys, os, json, argparse
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import LEARNER, Bufs, force_mask, KINDS, act

a = argparse.ArgumentParser()
a.add_argument('ckpt'); a.add_argument('--arm', choices=['none', 'early', 'late'], required=True)
a.add_argument('--games', type=int, default=8192)
a.add_argument('--vram-gb', type=float, required=True)
a.add_argument('--seed-base', type=int, default=fe.SEED_BASE)
a.add_argument('--out', required=True)
args = a.parse_args()
CHUNK = fe.CHUNK
KIND_OF = ppo.KIND_OF.numpy()
HP = np.nonzero(np.isin(KIND_OF, [KINDS.index('hero_power'), KINDS.index('hero_power2')]))[0]
END = bgpy.END_TURN
dev = torch.device('cuda')
torch.cuda.set_per_process_memory_fraction(
    min(1.0, args.vram_gb / (torch.cuda.get_device_properties(0).total_memory / 2**30)), 0)
info = fe.load_ckpt(args.ckpt, dev); net = info['net']
ppo.FORCE_DARK = info['force_dark_gift']; ppo.FORCE_TURN2 = False
ppo.OLD_BOARDS = info['old_boards']; ppo.REAL_LOBBY_FILTER = True
real, _ = real_boards.load(ROOT + '/boards/frozen/real-2026-11-10')
HPt = torch.from_numpy(HP).to(dev)
hp_only = torch.zeros(bgpy.N_ACTIONS, dtype=torch.bool, device=dev); hp_only[HPt] = True

rows = []
for chunk in range((args.games + CHUNK - 1) // CHUNK):
    n = min(CHUNK, args.games - chunk * CHUNK)
    env = bgpy.VecEnv(n, args.seed_base + chunk, fe.LINEUP, real_boards=real,
                      old_boards=info['old_boards'], real_lobby_filter=True,
                      real_lag=0, real_lag_from=1)
    heroes = env.hero_names()
    b = Bufs(n); env.kinds(b.kinds); done = np.zeros(n, bool)
    hero = np.zeros(n, np.int64); last_turn = np.full(n, -1); pressed = np.zeros(n, bool)
    forced = np.zeros(n, np.int64); presses = np.zeros(n, np.int64)
    torch.manual_seed(args.seed_base + chunk)
    while not done.all():
        env.observe(b.obs, b.mask, b.seat, b.kind)
        turns = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
        learner = (~done) & (b.kind == LEARNER)
        newt = learner & (turns != last_turn)
        pressed[newt] = False; last_turn[newt] = turns[newt]
        h = b.obs[:, bgpy.SCALAR_HERO].astype(np.int64)
        hero = np.where(learner & (h > 0), h, hero)
        o = torch.from_numpy(b.obs).to(dev)
        mask = force_mask(o, torch.from_numpy(b.mask).to(dev))
        can = torch.from_numpy(learner & ~pressed).to(dev) & mask[:, HPt].any(1)
        if args.arm == 'early' and can.any():
            mask = torch.where(can[:, None], mask & hp_only[None, :], mask)
            forced += can.cpu().numpy()
        acts, _, _, _ = act(net, o, mask)
        if args.arm == 'late':
            redo = can & (acts == END)
            if redo.any():
                acts2, _, _, _ = act(net, o, mask & hp_only[None, :])
                acts = torch.where(redo, acts2, acts)
                forced += redo.cpu().numpy()
        acts_np = acts.cpu().numpy()
        hit = learner & np.isin(acts_np, HP)
        pressed |= hit; presses += hit
        kk = b.kinds.copy()
        env.step(acts_np, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kk[e, s] != LEARNER or done[e]: continue
            rows.append(dict(game=chunk * CHUNK + int(e), hero=heroes[hero[e] - 1] if hero[e] else None,
                             place=int(b.places[e, s]), forced=int(forced[e]), presses=int(presses[e])))
            done[e] = True
        if b.reset.any(): env.kinds(b.kinds)
    print(f'{args.arm} chunk {chunk}: {len(rows)} games', flush=True)
os.makedirs(os.path.dirname(args.out), exist_ok=True)
json.dump(dict(ckpt=args.ckpt, arm=args.arm, rows=sorted(rows, key=lambda r: r['game'])), open(args.out, 'w'))
