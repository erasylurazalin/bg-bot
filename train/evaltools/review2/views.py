"""Dump the bot's Tavern views (cards on offer, what it bought, board at turn start) for the buy-prior comparison."""
import sys, os, json
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))); sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import LEARNER, Bufs, force_mask, KINDS
from model import masked
KIND_OF = ppo.KIND_OF.numpy(); END = bgpy.END_TURN; SF = bgpy.SLOT_FEATURES
ckpt, lag, games, out = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
dev = torch.device('cuda'); torch.cuda.set_per_process_memory_fraction(3.0 / (torch.cuda.get_device_properties(0).total_memory / 2**30), 0)
info = fe.load_ckpt(ckpt, dev); net = info['net']; ppo.FORCE_DARK = True; ppo.FORCE_TURN2 = False
real, _ = real_boards.load(ROOT + '/boards/frozen/real-2026-11-10')
BUY0 = int(np.nonzero(KIND_OF == KINDS.index('buy'))[0][0]); BUYS = int(np.nonzero(KIND_OF == KINDS.index('buy_spell'))[0][0])
ROLL = int(np.nonzero(KIND_OF == KINDS.index('roll'))[0][0])
views = []
def slot(o, start, i):
    b = start + i * SF
    return (int(o[b + bgpy.SLOT_CARD_ID]) - 1, bool(o[b + bgpy.SLOT_GOLDEN] > 0)) if o[b + bgpy.SLOT_PRESENT] > 0 else None
for chunk in range(games // 512):
    n = 512
    env = bgpy.VecEnv(n, 20261110 + chunk, [1, 0, 0, 0, 0, 7], real_boards=real, old_boards='keep', real_lobby_filter=True, real_lag=lag, real_lag_from=7)
    names = env.card_names()
    b = Bufs(n); env.kinds(b.kinds); done = np.zeros(n, bool); torch.manual_seed(chunk)
    cur = [None] * n; turn_board = [None] * n; last_turn = np.zeros(n, int); fresh = np.ones(n, bool)
    def close(e, how):
        if cur[e] is not None:
            cur[e]['ended'] = how; views.append(cur[e]); cur[e] = None
    while not done.all():
        env.observe(b.obs, b.mask, b.seat, b.kind)
        turns = b.obs[:, bgpy.SCALAR_TURN].astype(int)
        o = torch.from_numpy(b.obs).to(dev); m = force_mask(o, torch.from_numpy(b.mask).to(dev))
        with torch.no_grad():
            acts = torch.distributions.Categorical(logits=masked(net(o, comp=None)[0], m)).sample().cpu().numpy()
        for e in np.nonzero(~done)[0]:
            ob = b.obs[e]; t = int(turns[e])
            if t != last_turn[e]:
                close(e, 'end'); last_turn[e] = t; fresh[e] = True
                turn_board[e] = [dict(name=names[c], golden=g) for c, g in filter(None, (slot(ob, bgpy.BOARD_START, i) for i in range(bgpy.BOARD)))]
            if fresh[e]:
                off = [names[c] for c, _ in filter(None, (slot(ob, bgpy.SHOP_START, i) for i in range(bgpy.SHOP)))]
                sp = slot(ob, bgpy.SHOP_SPELL_START, 0)
                cur[e] = dict(game=chunk * 512 + int(e), turn=t, board=turn_board[e], offered=off, spell=names[sp[0]] if sp else None, bought=[])
                fresh[e] = False
            a = int(acts[e])
            if BUY0 <= a < BUY0 + bgpy.SHOP:
                s = slot(ob, bgpy.SHOP_START, a - BUY0)
                if s: cur[e]['bought'].append(names[s[0]])
            elif a == BUYS:
                s = slot(ob, bgpy.SHOP_SPELL_START, 0)
                if s: cur[e]['bought'].append(names[s[0]])
            elif a == ROLL:
                close(e, 'roll'); fresh[e] = True
        kk = b.kinds.copy()
        env.step(acts, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kk[e, s] == LEARNER and not done[e]: close(int(e), 'end'); done[e] = True
        if b.reset.any(): env.kinds(b.kinds)
json.dump(views, open(out, 'w'))
nb = sum(1 for v in views if v['bought']); print(out, len(views), 'views,', nb, 'with a buy')
