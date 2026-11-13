"""Report for hp_check.py rows: per hero, the power, turns it was legal, share of those turns
the bot pressed it, presses a turn. Usage: hp_report.py base.json [other.json] > report.md"""
import json, sys, tomllib, collections, os
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
H = tomllib.load(open(ROOT + '/heroes.toml', 'rb'))
pw = {}
for h in H.get('hero', []):
    p = h.get('power', {})
    tag = 'passive' if p.get('passive') else f"cost {p.get('cost', 0)}"
    pw[h['name']] = f"{p.get('name', '?')} ({tag})"
def per_hero(path):
    d = json.load(open(path)); g = collections.defaultdict(lambda: [0, 0, 0, 0, 0, 0.0, 0])
    for r in d['rows']:
        x = g[r['hero']]; x[0] += 1
        for t, v in r['turns'].items():
            x[1] += 1; x[2] += v[0]; x[3] += 1 if (v[0] and v[1]) else 0; x[4] += v[1]; x[5] += v[4]; x[6] += v[5]
    return g
runs = [(os.path.basename(p)[:-5], per_hero(p)) for p in sys.argv[1:]]
base = runs[0][1]
print('# Which hero powers the bot presses\n')
print('All turns, 1 learner vs 7 real, lag 0, 2,048 games, sampled at temp 1. "legal" = the power was pressable at some step of the turn; "pressed" = share of those turns it was pressed; "p" = mean policy probability of pressing at a step where it was legal.\n')
for name, g in runs:
    T = sum(x[2] for x in g.values()); P = sum(x[3] for x in g.values())
    print(f"- {name}: power legal on {T} turns, pressed on {P} ({P/max(1,T):.1%})")
print()
hdr = '| hero | power | games | turns legal |' + ''.join(f' {n} pressed | {n} a turn | {n} p |' for n, _ in runs)
print(hdr); print('|' + '---|' * (4 + 3 * len(runs)))
for h in sorted(base, key=lambda h: -(base[h][3] / max(1, base[h][2]))):
    x = base[h]
    if x[0] < 15: continue
    s = f"| {h} | {pw.get(h, '?')} | {x[0]} | {x[2]/max(1,x[1]):.0%} |"
    for _, g in runs:
        y = g.get(h)
        s += f" {y[3]/max(1,y[2]):.0%} | {y[4]/max(1,y[1]):.2f} | {y[5]/max(1,y[6]):.3f} |" if y and y[2] else " not legal | | |"
    print(s)
