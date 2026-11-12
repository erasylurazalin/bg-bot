"""Review harness: fixed-seed games (one game per seed, 1 learner vs 7 real) with options,
recording per-turn stats of the learner. Eval only."""
import sys, os, json, argparse, re
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import LEARNER, Bufs, force_mask, KINDS
from model import masked
KIND_OF = ppo.KIND_OF.numpy()
END = bgpy.END_TURN
a = argparse.ArgumentParser()
a.add_argument('ckpt'); a.add_argument('--games', type=int, default=1024)
a.add_argument('--rlag', dest='lag', type=int, default=0); a.add_argument('--rlag-from', dest='lag_from', type=int, default=7)
a.add_argument('--temp', type=float, default=1.0)
a.add_argument('--rules', default='')   # key=value;key=value on rules.toml
a.add_argument('--explicit-toml', action='store_true')
a.add_argument('--out', default=None); a.add_argument('--vram', type=float, default=3.0)
a.add_argument('--seed-base', type=int, default=20261110)
a.add_argument('--tag', default='')
a.add_argument('--no-level-from', type=int, default=0); a.add_argument('--no-level-turn2', action='store_true')
a.add_argument('--prior', default=None); a.add_argument('--beta', type=float, default=0.0); a.add_argument('--beta-roll', type=float, default=0.0); a.add_argument('--prior-from', type=int, default=1)
a.add_argument('--no-moves', action='store_true'); a.add_argument('--spend', action='store_true')
args = a.parse_args()
dev = torch.device('cuda')
tot_gb = torch.cuda.get_device_properties(0).total_memory / 2**30
torch.cuda.set_per_process_memory_fraction(min(1.0, args.vram / tot_gb), 0)
info = fe.load_ckpt(args.ckpt, dev); net = info['net']
ppo.FORCE_DARK = info['force_dark_gift']; ppo.FORCE_TURN2 = False
ppo.OLD_BOARDS = info['old_boards']; ppo.REAL_LOBBY_FILTER = True
real, _ = real_boards.load(ROOT + '/boards/frozen/real-2026-11-10')
kw = {}
if args.rules or args.explicit_toml:
    rules = open(ROOT + '/rules.toml').read()
    for kv in filter(None, args.rules.split(';')):
        k, v = kv.split('=', 1)
        rules, n = re.subn(rf'(?m)^{re.escape(k)}\s*=.*$', f'{k} = {v}', rules)
        assert n == 1, f'rule {k} not found'
    kw = dict(cards_toml=open(ROOT + '/cards.toml').read(), rules_toml=rules,
              heroes_toml=open(ROOT + '/heroes.toml').read())
PRIOR = None
if args.prior:
    sys.path.insert(0, ROOT + '/train/evaltools'); import buy_prior as bp
    pk = torch.load(args.prior, map_location='cpu', weights_only=False)
    PRIOR = bp.BuyPrior(pk['state_dict']['card.weight'].shape[0], pk['state_dict']['meta.weight'].shape[0], False); PRIOR.load_state_dict(pk['state_dict']); PRIOR = PRIOR.to(dev).eval()
    PIDX = None
    SF = bgpy.SLOT_FEATURES
    SHOP_COLS = [bgpy.SHOP_START + i * SF for i in range(bgpy.SHOP)] + [bgpy.SHOP_SPELL_START]
    BOARD_COLS = [bgpy.BOARD_START + i * SF for i in range(bgpy.BOARD)]
    BUY0 = int(np.nonzero(KIND_OF == KINDS.index('buy'))[0][0]); BUYS = int(np.nonzero(KIND_OF == KINDS.index('buy_spell'))[0][0]); ROLLA = int(np.nonzero(KIND_OF == KINDS.index('roll'))[0][0])
    BUY_ACTS = torch.tensor([BUY0 + i for i in range(bgpy.SHOP)] + [BUYS], device=dev)
def prior_bonus(o, env):
    global PIDX
    if PIDX is None:
        names = env.card_names(); PIDX = torch.tensor([0] + [pk['card_idx'].get(n, 1) for n in names], device=dev)
    sc = torch.tensor(SHOP_COLS, device=dev); bc = torch.tensor(BOARD_COLS, device=dev)
    sid = (o[:, sc + bgpy.SLOT_CARD_ID] * (o[:, sc + bgpy.SLOT_PRESENT] > 0)).long()
    bid = (o[:, bc + bgpy.SLOT_CARD_ID] * (o[:, bc + bgpy.SLOT_PRESENT] > 0)).long()
    offers = PIDX[sid]; board = PIDX[bid]; bmask = (bid > 0).float()
    turns = o[:, bgpy.SCALAR_TURN].long().clamp(0, pk['max_turn'])
    with torch.no_grad():
        lg, rf = PRIOR(offers, board, bmask, torch.zeros((len(o), 1), dtype=torch.long, device=dev), torch.zeros((len(o), 1), device=dev), turns)
    bonus = torch.zeros((len(o), bgpy.N_ACTIONS), device=dev)
    on = (o[:, bgpy.SCALAR_TURN] >= args.prior_from).float()[:, None]
    bonus[:, BUY_ACTS] = args.beta * lg * (sid > 0).float() * on
    bonus[:, ROLLA] = args.beta_roll * rf * on[:, 0]
    return bonus
MAXT = 20
agg = {k: np.zeros(MAXT + 1) for k in ['turns', 'steps', 'pend', 'ent', 'legal', 'gold_end', 'total_end', 'ended', 'v_start', 'gold_start', 'starts']}
kinds = np.zeros((MAXT + 1, len(KINDS)))
rows = []
for chunk in range((args.games + 511) // 512):
    n = min(512, args.games - chunk * 512)
    env = bgpy.VecEnv(n, args.seed_base + chunk, [1, 0, 0, 0, 0, 7], real_boards=real, old_boards=info['old_boards'],
                      real_lobby_filter=True, real_lag=args.lag, real_lag_from=args.lag_from, **kw)
    b = Bufs(n); env.kinds(b.kinds); done = np.zeros(n, bool); last_turn = np.zeros(n, int)
    torch.manual_seed(args.seed_base + chunk)
    while not done.all():
        env.observe(b.obs, b.mask, b.seat, b.kind)
        turns = np.minimum(b.obs[:, bgpy.SCALAR_TURN].astype(int), MAXT)
        o = torch.from_numpy(b.obs).to(dev)
        mask = force_mask(o, torch.from_numpy(b.mask).to(dev))
        if args.no_level_from or args.no_level_turn2:
            LV = int(np.nonzero(KIND_OF == KINDS.index('level'))[0][0])
            block = torch.zeros(len(o), dtype=torch.bool, device=dev)
            if args.no_level_from: block |= o[:, bgpy.SCALAR_TIER] >= args.no_level_from
            if args.no_level_turn2: block |= o[:, bgpy.SCALAR_TURN] == 2
            other = mask.clone(); other[:, LV] = False
            block &= other.any(1)
            mask = mask.clone(); mask[block, LV] = False
        if args.no_moves:
            mv = torch.from_numpy(KIND_OF == KINDS.index('move')).to(dev)
            other = (mask & ~mv[None, :]).any(1)
            mask = torch.where(other[:, None], mask & ~mv[None, :], mask)
        if args.spend:
            ROLL = int(np.nonzero(KIND_OF == KINDS.index('roll'))[0][0])
            can = mask[:, ROLL] & (o[:, bgpy.SCALAR_GOLD] >= 1) & (o[:, bgpy.SCALAR_ACTIONS_LEFT] > 40)
            mask = mask.clone(); mask[can, END] = False
        with torch.no_grad():
            logits, value = net(o, comp=None)
            if PRIOR is not None: logits = logits + prior_bonus(o, env)
            ml = masked(logits, mask)
            p = torch.softmax(ml, 1)
            ent = -(p * torch.log(p.clamp_min(1e-12))).sum(1)
            if args.temp <= 0:
                acts = ml.argmax(1)
            else:
                acts = torch.distributions.Categorical(logits=ml / args.temp).sample()
        acts_np = acts.cpu().numpy(); pend = p[:, END].cpu().numpy(); ent = ent.cpu().numpy()
        legal = mask.sum(1).cpu().numpy(); val = value.cpu().numpy()
        live = ~done
        new = live & (turns != last_turn)
        np.add.at(agg['starts'], turns[new], 1); np.add.at(agg['v_start'], turns[new], val[new])
        np.add.at(agg['gold_start'], turns[new], b.obs[new, bgpy.SCALAR_GOLD])
        last_turn[new] = turns[new]
        np.add.at(agg['steps'], turns[live], 1); np.add.at(agg['pend'], turns[live], pend[live])
        np.add.at(agg['ent'], turns[live], ent[live]); np.add.at(agg['legal'], turns[live], legal[live])
        np.add.at(kinds, (turns[live], KIND_OF[acts_np[live]]), 1)
        ending = live & ((acts_np == END) | (b.obs[:, bgpy.SCALAR_ACTIONS_LEFT] <= 1))
        for e in np.nonzero(ending)[0]:
            st = env.seats(int(e))[int(b.seat[e])]
            agg['total_end'][turns[e]] += sum(m[2] + m[3] for m in st[5])
        np.add.at(agg['gold_end'], turns[ending], b.obs[ending, bgpy.SCALAR_GOLD]); np.add.at(agg['ended'], turns[ending], 1)
        kk = b.kinds.copy()
        env.step(acts_np, b.places, b.tier_turns, b.tribes, b.reset)
        for e, s in zip(*np.nonzero(b.places)):
            if kk[e, s] != LEARNER or done[e]: continue
            rows.append(dict(game=chunk * 512 + int(e), place=int(b.places[e, s]), ko=int(turns[e])))
            done[e] = True
        if b.reset.any(): env.kinds(b.kinds)
pl = np.array([r['place'] for r in sorted(rows, key=lambda r: r['game'])])
se = pl.std(ddof=1) / np.sqrt(len(pl))
head = f"{args.tag or os.path.basename(os.path.dirname(args.ckpt))} lag {args.lag} temp {args.temp} rules [{args.rules}] nomoves {args.no_moves} spend {args.spend} nolevelfrom {args.no_level_from} noL2 {args.no_level_turn2} prior beta {args.beta} roll {args.beta_roll} from {args.prior_from} games {len(pl)}"
print(f"== {head}\n   place {pl.mean():.3f} (+-{1.96*se:.3f})  top4 {(pl<=4).mean():.1%}  firsts {(pl==1).sum()}  8th {(pl==8).mean():.1%}")
print('   turn  games  acts  P(end)  entropy  legal  gold@start  gold@end  total@end  V@start | buy spell sell play roll level hero move')
ki = {k: i for i, k in enumerate(KINDS)}
for t in range(1, MAXT + 1):
    g = agg['ended'][t]
    if g < 20: continue
    st = max(1, agg['steps'][t])
    kc = lambda k: kinds[t, ki[k]] / g
    print(f"   {t:4d} {int(g):6d} {agg['steps'][t]/g:5.1f} {agg['pend'][t]/st:7.3f} {agg['ent'][t]/st:8.2f} {agg['legal'][t]/st:6.1f} {agg['gold_start'][t]/max(1,agg['starts'][t]):10.1f} {agg['gold_end'][t]/g:9.2f} {agg['total_end'][t]/g:10.0f} {agg['v_start'][t]/max(1,agg['starts'][t]):8.3f} | {kc('buy'):.2f} {kc('buy_spell'):.2f} {kc('sell'):.2f} {kc('play'):.2f} {kc('roll'):.2f} {kc('level'):.2f} {kc('hero_power'):.2f} {kc('move'):.2f}")
if args.out:
    json.dump(dict(head=head, places=pl.tolist(), rows=rows, agg={k: v.tolist() for k, v in agg.items()}, kinds=kinds.tolist()), open(args.out, 'w'))
