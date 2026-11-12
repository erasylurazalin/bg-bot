"""Counts behind docs/notes/mining/sure-plays.md.

A turn is the unit. "Offered" means the card was in the Tavern at the turn's
start or in any step's after state, while the state had enough Gold for it.
"""
import collections
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib


def offered_affordable(turn, name):
    states = [turn['start']] + [a['after'] for a in turn['actions'] if a.get('after')]
    return any(
        st.get('gold', 0) >= lib.cost_of(c)
        for st in states
        for c in st.get('tavern') or []
        if c.get('name') == name
    )


def bought(turn, name):
    return any(a['step'] == 'buy' and a.get('card', {}).get('name') == name
               for a in turn['actions'])


def played(turn, name):
    return any(a['step'] == 'play' and a.get('card', {}).get('name') == name
               for a in turn['actions'])


def sold(turn, name):
    return any(a['step'] == 'sell' and a.get('card', {}).get('name') == name
               for a in turn['actions'])


def level_rows(games):
    rows = [
        ('turn 2, tier 1, 4 Gold, upgrade costs 4', lambda t, s, u: t['turn'] == 2 and s.get('tier') == 1 and s.get('gold') == 4 and u == 4),
        ('turn 5, tier 2, upgrade affordable', lambda t, s, u: t['turn'] == 5 and s.get('tier') == 2),
        ('turn 6, tier 3, upgrade affordable', lambda t, s, u: t['turn'] == 6 and s.get('tier') == 3),
        ('turn 8, tier 4, upgrade affordable', lambda t, s, u: t['turn'] == 8 and s.get('tier') == 4),
    ]
    for label, cond in rows:
        made = could = 0
        examples = []
        for g in games:
            for t in g['turns']:
                s = t['start']
                u = s.get('upgrade_cost')
                if u is None or s.get('gold', 0) < u or not cond(t, s, u):
                    continue
                could += 1
                if any(a['step'] == 'level' for a in t['actions']):
                    made += 1
                    examples.append((g['source'], g['game'], t['turn']))
        print(f'{label:44s} {made:3d}/{could:3d} {made / could:5.0%} {examples[:2]}')


def shell(games):
    made = could = 0
    examples = []
    for g in games:
        for t in g['turns']:
            s = t['start']
            if s.get('won_last_combat') is not False or s.get('gold', 0) < 3:
                continue
            if not offered_affordable(t, 'Tortollan Blue Shell'):
                continue
            could += 1
            if bought(t, 'Tortollan Blue Shell'):
                made += 1
                examples.append((g['source'], g['game'], t['turn']))
    print(f'Blue Shell after a loss: {made}/{could} {made / could:.0%} {examples[:2]}')


def offers(games, names):
    st = {n: {'off': 0, 'buy': 0, 'ex': []} for n in names}
    for g in games:
        for t in g['turns']:
            for n in names:
                if not offered_affordable(t, n):
                    continue
                st[n]['off'] += 1
                if bought(t, n):
                    st[n]['buy'] += 1
                    if len(st[n]['ex']) < 2:
                        st[n]['ex'].append((g['source'], g['game'], t['turn']))
    for n in names:
        s = st[n]
        print(f'{n:28s} {s["buy"]:3d}/{s["off"]:3d} {s["buy"] / s["off"]:5.0%} {s["ex"]}')


def spells(games):
    bought_n = played_n = 0
    per = collections.Counter()
    per_buy = collections.Counter()
    ex = collections.defaultdict(list)
    for g in games:
        for t in g['turns']:
            names = {a['card']['name'] for a in t['actions'] if a['step'] == 'buy' and a.get('spell')}
            cast = {a.get('card', {}).get('name') for a in t['actions'] if a['step'] == 'play'}
            for n in names:
                bought_n += 1
                per_buy[n] += 1
                if n in cast:
                    played_n += 1
                    per[n] += 1
                    if len(ex[n]) < 2:
                        ex[n].append((g['source'], g['game'], t['turn']))
    print(f'spells bought {bought_n}, cast same turn {played_n} ({played_n / bought_n:.0%})')
    for n, c in per_buy.most_common():
        if c >= 5:
            print(f'{n:28s} {per[n]:3d}/{c:3d} {per[n] / c:5.0%} {ex[n]}')


def cycles(games, names):
    for n in names:
        off = buy = bps = 0
        net = 0
        ex = []
        for g in games:
            for t in g['turns']:
                if offered_affordable(t, n):
                    off += 1
                if not bought(t, n):
                    continue
                buy += 1
                if played(t, n) and sold(t, n):
                    bps += 1
                    net += sum(a.get('paid', {}).get('gold', 0) for a in t['actions']
                               if a.get('card', {}).get('name') == n and a['step'] in ('buy', 'sell'))
                    if len(ex) < 2:
                        ex.append((g['source'], g['game'], t['turn']))
        print(f'{n:24s} offers {off:3d} buys {buy:3d} bps {bps:3d} '
              f'{bps / max(1, buy):5.0%} of buys, net cost {net / max(1, bps):3.1f} Gold {ex}')


def hero_powers(games):
    opp = collections.Counter()
    used = collections.Counter()
    ex = collections.defaultdict(list)
    for g in games:
        for t in g['turns']:
            s = t['start']
            for hp in s.get('hero_powers') or []:
                n = hp.get('name')
                if hp.get('used_this_turn', 0) != 0 or hp.get('cost', 0) > s.get('gold', 0):
                    continue
                opp[n] += 1
                if any(a['step'] == 'hero_power' for a in t['actions']):
                    used[n] += 1
                    if len(ex[n]) < 2:
                        ex[n].append((g['source'], g['game'], t['turn']))
    for n, c in opp.most_common():
        if c >= 5 and used[n] / c >= 0.8:
            print(f'{n:28s} {used[n]:3d}/{c:3d} {used[n] / c:5.0%} {ex[n]}')


def triples(games):
    def minions(state):
        return [c for w in ('board', 'hand') for c in state.get(w) or []
                if c.get('kind') != 'spell' and c.get('name')]
    made = could = 0
    for g in games:
        for t in g['turns']:
            s = t['start']
            owned = collections.Counter(c['name'] for c in minions(s)
                                        if not c.get('golden') and not c.get('no_reward'))
            tav = collections.Counter(c.get('name') for c in s.get('tavern') or [] if not c.get('golden'))
            end = t.get('end') or s
            for n, count in owned.items():
                if count < 2:
                    continue
                need = 3 - count
                if tav[n] >= need and s.get('gold', 0) >= 3 * need:
                    could += 1
                    if any(c['name'] == n and c.get('golden') for c in minions(end)):
                        made += 1
    print(f'triple completable at start: {made}/{could} {made / could:.0%}')


def freeze(games):
    turns = froze = match = den = 0
    for g in games:
        ts = g['turns']
        for i, t in enumerate(ts):
            turns += 1
            if not any(a['step'] == 'freeze' and a.get('on') for a in t['actions']):
                continue
            froze += 1
            end = t.get('end') or t['start']
            frozen = {c['name'] for c in end.get('tavern') or []}
            if i + 1 < len(ts) and frozen:
                den += 1
                nxt = {a.get('card', {}).get('name') for a in ts[i + 1]['actions'] if a['step'] == 'buy'}
                if frozen & nxt:
                    match += 1
    print(f'freeze turns {froze}/{turns} ({froze / turns:.0%}); next turn bought a frozen card {match}/{den} ({match / den:.0%})')


if __name__ == '__main__':
    games = lib.load_games()
    print('== level ==')
    level_rows(games)
    print('\n== shell ==')
    shell(games)
    print('\n== offers ==')
    offers(games, ['Tavern Coin', 'Leaf Through the Pages', 'Wealthy Bounty', "Fandral's Fortune",
                   'Careful Investment', 'Boundless Potential'])
    print('\n== spells ==')
    spells(games)
    print('\n== cycles ==')
    cycles(games, ['Fearless Foodie', 'Snare Trapper', 'Sly Infiltrator', 'Refreshing Anomaly',
                   'Rodeo Performer', 'Snarky Shark', 'Tavern Tempest', 'Iron Groundskeeper',
                   'Sprightly Scarab', 'Veteran Brigand', 'Crater Miner', 'Patient Scout',
                   'Razorfen Geomancer', 'Greedy Conniver'])
    print('\n== hero powers (>=80%) ==')
    hero_powers(games)
    print('\n== triples ==')
    triples(games)
    print('\n== freeze ==')
    freeze(games)
