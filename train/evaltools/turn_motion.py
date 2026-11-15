"""Rolls, buys, sells and cycles a turn on turns 10 to 13 in normal games (plan-2026-11-13
11.5: every gate report gives these). A cycle is a minion sold in the same turn it was
bought (matched by card name). `cycles_fx` counts only cycles of a minion that has a
Battlecry, or an effect that fires when it is bought, played or sold, read from the
card's effect data in cards.toml. The report lists the ten most cycled cards per
checkpoint with their counts and whether they count. Same games as
train/fixed_eval.py (its seeds, chunks, lineup 1 learner vs 7 real, lag 0, sampled).
Eval only.

    python train/evaltools/turn_motion.py A.pt B.pt ... --games 512 --vram-gb 3 [--cpu]
"""
import sys, os, json, argparse, collections, tomllib
CWD = os.getcwd()
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT + '/train'); os.chdir(ROOT + '/train')
import numpy as np, torch, bgpy, real_boards, ppo, fixed_eval as fe
from ppo import LEARNER, Bufs, force_mask, KINDS, act

# The card fields that fire when the minion is bought, played or sold. A choose
# one is a play effect as much as a Battlecry; Activate and the turn/combat
# triggers are not. The engine reads the same fields from cards.toml.
EFFECT_KEYS = ('battlecry', 'choose_one', 'on_play_mech',
               'on_play_or_magnetize_mech', 'on_sell')


def effect_cards(path):
    """The minion names with a buy, play or sell effect, from cards.toml."""
    with open(path, 'rb') as f:
        data = tomllib.load(f)
    names = set()
    for section in ('minion', 'token'):
        for card in data.get(section, []):
            if any(card.get(key) for key in EFFECT_KEYS):
                names.add(card['name'])
    return names


def note_cycle(tot, by_card, name, turn, effects):
    """Count one cycle of `name` on `turn`, once for all cycles and once for
    the guarded measure when the card has a buy, play or sell effect."""
    tot[('cycles', turn)] += 1
    by_card[name] += 1
    if name in effects:
        tot[('cycles_fx', turn)] += 1
        return True
    return False


def card(obs, start, slot, names):
    cid = int(obs[start + slot * bgpy.SLOT_FEATURES + bgpy.SLOT_CARD_ID])
    return names[cid - 1] if cid > 0 else None


def run(path, args, effects, dev, real, buy, sell):
    info = fe.load_ckpt(path, dev); net = info['net']
    ppo.FORCE_DARK = info['force_dark_gift']; ppo.FORCE_TURN2 = False
    ppo.OLD_BOARDS = info['old_boards']; ppo.REAL_LOBBY_FILTER = True
    tot = collections.Counter()
    by_card = collections.Counter()
    for chunk in range((args.games + fe.CHUNK - 1) // fe.CHUNK):
        n = min(fe.CHUNK, args.games - chunk * fe.CHUNK)
        env = bgpy.VecEnv(n, fe.SEED_BASE + chunk, fe.LINEUP, real_boards=real,
                          old_boards=info['old_boards'], real_lobby_filter=True,
                          real_lag=args.real_lag, real_lag_from=1)
        names = env.card_names()
        b = Bufs(n); env.kinds(b.kinds); done = np.zeros(n, bool)
        last = np.full(n, -1); bought = [collections.Counter() for _ in range(n)]
        torch.manual_seed(fe.SEED_BASE + chunk)
        while not done.all():
            env.observe(b.obs, b.mask, b.seat, b.kind)
            turns = b.obs[:, bgpy.SCALAR_TURN].astype(np.int64)
            learner = (~done) & (b.kind == LEARNER)
            o = torch.from_numpy(b.obs).to(dev)
            acts, _, _, _ = act(net, o, force_mask(o, torch.from_numpy(b.mask).to(dev)))
            an = acts.cpu().numpy()
            for e in np.nonzero(learner & (turns >= args.t_lo) & (turns <= args.t_hi))[0]:
                t = int(turns[e])
                if t != last[e]:
                    last[e] = t; bought[e] = collections.Counter(); tot[('turns', t)] += 1
                x = int(an[e])
                if ppo.KIND_OF[x] == KINDS.index('roll'):
                    tot[('rolls', t)] += 1
                elif x in buy:
                    nm = card(b.obs[e], bgpy.SHOP_START, x - buy[0], names)
                    tot[('buys', t)] += 1
                    if nm: bought[e][nm] += 1
                elif x in sell:
                    nm = card(b.obs[e], bgpy.BOARD_START, x - sell[0], names)
                    tot[('sells', t)] += 1
                    if nm and bought[e][nm] > 0:
                        bought[e][nm] -= 1
                        note_cycle(tot, by_card, nm, t, effects)
            kinds = b.kinds.copy()
            env.step(an, b.places, b.tier_turns, b.tribes, b.reset)
            for e, s in zip(*np.nonzero(b.places)):
                if kinds[e, s] == LEARNER and not done[e]:
                    done[e] = True; tot[('place', 0)] += int(b.places[e, s]); tot[('games', 0)] += 1
            if b.reset.any(): env.kinds(b.kinds)
    return tot, by_card


def main():
    a = argparse.ArgumentParser()
    a.add_argument('ckpts', nargs='+')
    a.add_argument('--games', type=int, default=512)
    a.add_argument('--turns', default='10,13')
    a.add_argument('--real-lag', type=int, default=0)
    a.add_argument('--vram-gb', type=float, required=True)
    a.add_argument('--cpu', action='store_true', help='run on the CPU')
    a.add_argument('--out', default=None)
    args = a.parse_args()
    args.t_lo, args.t_hi = (int(x) for x in args.turns.split(','))
    dev = torch.device('cpu' if args.cpu else 'cuda')
    if dev.type == 'cuda':
        torch.cuda.set_per_process_memory_fraction(
            min(1.0, args.vram_gb / (torch.cuda.get_device_properties(0).total_memory / 2**30)), 0)
    real, _ = real_boards.load(str(fe.FROZEN))
    kind_of = ppo.KIND_OF.numpy()
    buy = np.nonzero(kind_of == KINDS.index('buy'))[0]
    sell = np.nonzero(kind_of == KINDS.index('sell'))[0]
    effects = effect_cards(os.path.join(ROOT, 'cards.toml'))

    L = [f'# Motion on turns {args.t_lo} to {args.t_hi} (lag {args.real_lag}, '
         f'{args.games} games, per turn played)\n',
         '| checkpoint | place | turns played | rolls | buys | sells | cycles | cycles_fx |',
         '|---|---|---|---|---|---|---|---|']
    res = {}
    for p in args.ckpts:
        tot, by_card = run(os.path.join(CWD, p), args, effects, dev, real, buy, sell)
        T = sum(tot[('turns', t)] for t in range(args.t_lo, args.t_hi + 1))
        f = lambda k: sum(tot[(k, t)] for t in range(args.t_lo, args.t_hi + 1)) / max(1, T)
        top = [{'name': name, 'count': count, 'counts_fx': name in effects}
               for name, count in by_card.most_common(10)]
        res[p] = dict(place=tot[('place', 0)] / tot[('games', 0)], turns=T,
                      rolls=f('rolls'), buys=f('buys'), sells=f('sells'),
                      cycles=f('cycles'), cycles_fx=f('cycles_fx'),
                      top_cycles=top)
        r = res[p]
        L.append(f"| `{p}` | {r['place']:.3f} | {T} | {r['rolls']:.2f} | "
                 f"{r['buys']:.2f} | {r['sells']:.2f} | {r['cycles']:.2f} | "
                 f"{r['cycles_fx']:.2f} |")
    for p in args.ckpts:
        r = res[p]
        L.append('')
        L.append(f'## `{p}` ten most cycled cards')
        L.append('')
        L.append('| card | cycles | counts for cycles_fx |')
        L.append('|---|---:|---|')
        for row in r['top_cycles']:
            L.append(f"| {row['name']} | {row['count']} | "
                     f"{'yes' if row['counts_fx'] else 'no'} |")
    print('\n'.join(L))
    if args.out:
        out = os.path.join(CWD, args.out); os.makedirs(out, exist_ok=True)
        open(out + '/report.md', 'w').write('\n'.join(L) + '\n')
        json.dump(res, open(out + '/results.json', 'w'), indent=1)


if __name__ == '__main__':
    main()
