"""Tier curve of every checkpoint in a fixed-eval results.json:
share levelling on turn 2, mean turn reaching tiers 3 to 6, tier at the end of turns 5, 7, 8.
Usage: python3 tiercurve.py train/runs/fixed-eval/NAME/results.json"""
import json, sys, numpy as np
d = json.load(open(sys.argv[1]))
for ck in d['checkpoints']:
    rows = ck['rows']
    t = np.array([r['tiers'] for r in rows]); ko = np.array([r['knocked_out_turn'] or 99 for r in rows]); pl = np.array([r['place'] for r in rows])
    def reach(k):
        x = t[:, k - 1]; return (x[x > 0].mean() if (x > 0).any() else float('nan')), (x > 0).mean()
    def tier_at(T):
        tt = np.zeros(len(rows))
        for k in range(6): tt = np.where((t[:, k] > 0) & (t[:, k] <= T), k + 1, tt)
        alive = ko > T; return tt[alive].mean()
    print(f"{ck['path']}: place {pl.mean():.3f}; levels on turn 2 {(t[:, 1] == 2).mean():.1%}; "
          f"turn reaching tier 3/4/5/6 {reach(3)[0]:.2f}/{reach(4)[0]:.2f}/{reach(5)[0]:.2f}/{reach(6)[0]:.2f} "
          f"(tier 5 reached in {reach(5)[1]:.0%}, tier 6 in {reach(6)[1]:.0%}); "
          f"tier at end of turn 5/7/8 {tier_at(5):.2f}/{tier_at(7):.2f}/{tier_at(8):.2f}")
