"""Print a run's metrics as a table: python train/status.py runs/NAME [rows]"""

import csv
import sys

COLS = [
    ("update", "update"), ("minutes", "min"), ("games", "games"),
    ("place_vs_rule", "vs7rule"), ("place_4v4_rule", "4v4rule"), ("place_vs_snapshot", "vs_old"),
    ("entropy", "entropy"), ("actions_per_turn", "act/trn"), ("tier_t6", "tier@6"), ("tier_t10", "tier@10"),
    ("act_buy", "buy"), ("act_sell", "sell"), ("act_play", "play"), ("act_move", "move"),
    ("act_roll", "roll"), ("act_freeze", "freeze"), ("act_level", "level"), ("act_end", "end"),
    ("explained_var", "expl_v"), ("approx_kl", "kl"),
]

rows = list(csv.DictReader(open(f"{sys.argv[1]}/metrics.csv")))
n = int(sys.argv[2]) if len(sys.argv) > 2 else 15
step = max(1, len(rows) // n)
shown = rows[::step]
if rows and shown[-1] is not rows[-1]:
    shown.append(rows[-1])
print(" ".join(h.rjust(7) for _, h in COLS))
for r in shown:
    print(" ".join(str(r.get(k, ""))[:7].rjust(7) for k, _ in COLS))
if rows:
    print("top buys now:", rows[-1]["top_buys"])
