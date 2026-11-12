import json, glob, tomllib, collections, sys
comps = tomllib.load(open('data/comps/comps.toml','rb'))['comp']
core = [(c['name'], {x['name']: x['weight'] for x in c['core']}) for c in comps]
def classify(names):
    names = set(names); best = None
    for cn, w in core:
        have = [n for n in w if n in names]
        frac = sum(w[n] for n in have) / sum(w.values())
        if len(have) >= 2 and frac >= 0.5 and (best is None or frac > best[1]):
            best = (cn, frac)
    return best[0] if best else None
def report(label, boards):  # boards: list of (names, place)
    n = len(boards); hits = collections.defaultdict(list)
    for names, place in boards:
        c = classify(names)
        hits[c].append(place)
    comp = sum(len(v) for k, v in hits.items() if k)
    cp = [p for k, v in hits.items() if k for p in v]; mp = hits[None]
    print(f"{label}: {n} boards, comp {comp} ({comp/n:.1%}), comp place {sum(cp)/max(len(cp),1):.2f}, no comp place {sum(mp)/max(len(mp),1):.2f}, firsts on comp {sum(1 for p in cp if p==1)} / all firsts {sum(1 for _,p in boards if p==1)}")
    top = sorted(((k, v) for k, v in hits.items() if k), key=lambda kv: -len(kv[1]))[:6]
    print('   ' + '; '.join(f"{k} {len(v)} @{sum(v)/len(v):.2f}" for k, v in top))
# Era: last own board in each game
era = []
for f in glob.glob('boards/real/*.json'):
    d = json.load(open(f))
    if not d.get('place'): continue
    own = [b for b in d['boards'] if b.get('who') in ('me', 'era', 'own', 'mine')]
    if not own: continue
    last = max(own, key=lambda b: b['turn'])
    era.append(([m['name'] for m in last['minions']], d['place']))
whos = collections.Counter(b.get('who') for f in glob.glob('boards/real/*.json') for b in json.load(open(f))['boards'])
print('who values', dict(whos))
report('Era final boards', era)
for lag in (0, 2):
    for c in ('r1109-bundle', 'r1111-lagmix', 'r1112-topheavy'):
        rows = json.load(open(f'{sys.argv[1]}/fb-{c}-lag{lag}.json'))['rows']
        bs = []
        for r in rows:
            if not r['boards']: continue
            t = max(r['boards'], key=int)
            bs.append(([m['name'] for m in r['boards'][t]], r['place']))
        pl = sum(r['place'] for r in rows) / len(rows)
        report(f'{c} lag {lag} (place {pl:.3f})', bs)
