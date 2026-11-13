"""Report for forced_press.py: paired place differences (arm minus none, negative is better)
overall, by the hero's base power cost (heroes.toml), and for the rarely-pressed group.
Also checks the none arm against the fixed eval's own games."""
import json, os, sys, tomllib, math, collections
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
D = ROOT + '/train/runs/forced-press'
RARE = ['Captain Hooktusk', 'Cookie the Cook', 'Forest Lord Cenarius', 'Heistbaron Togwaggle',
        'Jandice Barov', "Lich Baz'hial", 'Maiev Shadowsong', 'Malygos', 'Mutanus the Devourer',
        'Patches the Pirate', 'Pyramad', 'The Jailer']
H = tomllib.load(open(ROOT + '/heroes.toml', 'rb'))
def cost_group(name):
    for h in H['hero']:
        if h['name'] == name:
            p = h.get('power', {})
            if p.get('passive'): return 'passive'
            c = p.get('cost', 0)
            return '0 Gold' if c == 0 else '1 Gold' if c == 1 else '2+ Gold'
    return 'unknown'
arm = {k: {r['game']: r for r in json.load(open(f'{D}/{k}.json'))['rows']} for k in ('none', 'early', 'late')}
games = sorted(set(arm['none']) & set(arm['early']) & set(arm['late']))
def stat(sel, k):
    d = [arm[k][g]['place'] - arm['none'][g]['place'] for g in sel]
    if len(d) < 2: return 'too few'
    m = sum(d) / len(d); sd = math.sqrt(sum((x - m) ** 2 for x in d) / (len(d) - 1)); h = 1.96 * sd / math.sqrt(len(d))
    return f"{m:+.3f} ({m-h:+.3f} to {m+h:+.3f})"
def mean(sel, k): return sum(arm[k][g]['place'] for g in sel) / max(1, len(sel))
def per_turn(sel, k, key):
    return sum(arm[k][g][key] for g in sel) / max(1, len(sel))
groups = collections.OrderedDict()
groups['all'] = games
groups['rarely pressed (12 heroes)'] = [g for g in games if arm['none'][g]['hero'] in RARE]
for cg in ('0 Gold', '1 Gold', '2+ Gold', 'passive', 'unknown'):
    groups[f'power {cg}'] = [g for g in games if cost_group(arm['none'][g]['hero']) == cg]
L = ['# T1: forced hero power press\n',
     f'BASE `train/runs/r1113-a1/best.pt`, lag 0, {len(games)} paired games on the fixed eval seeds. Differences are arm minus normal play, negative is better, with 95% ranges. "Presses" are hero power presses a game.\n',
     '| group | games | normal place | early place | early minus normal | late place | late minus normal | presses normal / early / late |',
     '|---|---|---|---|---|---|---|---|']
for name, sel in groups.items():
    if not sel: continue
    L.append(f"| {name} | {len(sel)} | {mean(sel,'none'):.3f} | {mean(sel,'early'):.3f} | {stat(sel,'early')} | {mean(sel,'late'):.3f} | {stat(sel,'late')} | "
             f"{per_turn(sel,'none','presses'):.1f} / {per_turn(sel,'early','presses'):.1f} / {per_turn(sel,'late','presses'):.1f} |")
L += ['\n## Rarely pressed, per hero (noise at this size; for reading only)\n',
      '| hero | games | normal | early minus normal | late minus normal |', '|---|---|---|---|---|']
for h in RARE:
    sel = [g for g in games if arm['none'][g]['hero'] == h]
    if sel: L.append(f"| {h} | {len(sel)} | {mean(sel,'none'):.2f} | {stat(sel,'early')} | {stat(sel,'late')} |")
fe = json.load(open(ROOT + '/train/runs/fixed-eval/r1113-a1-lag0/results.json'))
ck = [c for c in fe['checkpoints'] if 'r1113-a1/best' in str(c.get('path', c.get('checkpoint', '')))]
if ck:
    rows = {r['game']: r['place'] for r in ck[0]['rows']}
    same = sum(1 for g in rows if g in arm['none'] and arm['none'][g]['place'] == rows[g])
    L.append(f"\nCheck: the normal arm matches the fixed eval's own place on {same} of {len(rows)} games (r1113-a1-lag0).")
print('\n'.join(L))
open(D + '/report.md', 'w').write('\n'.join(L) + '\n')
