"""Print a run's metrics as a table: python train/status.py runs/NAME [rows]"""

import csv
import sys

COLS = [
    ("update", "update"), ("minutes", "min"), ("games", "games"),
    ("place_vs_std", "vs7std"), ("place_vs_rush", "vs7rush"),
    ("place_4v4_std", "4v4std"), ("place_4v4_rush", "4v4rush"),
    ("place_4v4_mix", "4v4mix"), ("place_vs_snapshot", "vs_old"),
    ("turn_t3", "trn@t3"), ("turn_t4", "trn@t4"), ("turn_t5", "trn@t5"),
    ("curve_econ_rush", "c_econ"), ("curve_rush", "c_rush"),
    ("curve_standard", "c_std"), ("curve_slow", "c_slow"),
    ("tribe_beast", "t_beast"), ("tribe_mech", "t_mech"), ("tribe_mixed", "t_mix"),
    ("place_beast", "p_beast"), ("place_mech", "p_mech"), ("place_mixed", "p_mix"),
    ("entropy", "entropy"), ("actions_per_turn", "act/trn"),
    ("tier_t6", "tier@6"), ("tier_t10", "tier@10"),
    ("act_buy", "buy"), ("act_buy_spell", "b_spl"), ("act_sell", "sell"),
    ("act_play", "play"), ("act_target", "target"), ("act_choose", "choose"),
    ("act_move", "move"), ("act_roll", "roll"), ("act_freeze", "freeze"),
    ("act_level", "level"), ("act_end", "end"),
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
