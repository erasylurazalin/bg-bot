#!/bin/bash
# Train for a while and stop: ./train/go.sh HOURS [RUN]
#
# Continues RUN (default r1025-attn, the real-board run of 2026-10-25) from
# its last checkpoint for HOURS more hours, then stops and leaves the GPU
# idle. It runs detached, so the terminal can close. A crash resumes from the
# last checkpoint (at most a few minutes lost), up to 5 times. Each start
# reloads boards/real, so games played since the last start join in.
#
#   ./train/go.sh 8          sleep: 8 hours
#   ./train/go.sh 3          class: 3 hours
#   ./train/stop.sh          stop now (loses at most the last few minutes)
#   tail train/runs/go.log   what it did
#
# The lineup is read from runs/<RUN>-lineup.txt if it exists, otherwise
# 2,2,0,0,0,4 (2 learner, 2 snapshot, 0 standard, 0 rush, 0 random, 4 real).
# Extra ppo.py flags come from runs/<RUN>-args.txt if it exists.
set -u
hours=${1:?usage: go.sh HOURS [RUN]}
run=${2:-r1025-attn}
cd "$(dirname "$0")"
log=runs/go.log
ck=runs/$run/ckpt.pt
py=../.venv/bin/python
# no checkpoint yet: a fresh run, which needs its lineup and args files
[ -f "$ck" ] || [ -f "runs/$run-args.txt" ] || [ -f "runs/$run-init.txt" ] || { echo "no checkpoint $ck and no runs/$run-args.txt"; exit 1; }
if pgrep -f "^$py ppo.py" >/dev/null || pgrep -f "^\.\./\.venv/bin/python ppo.py" >/dev/null; then
  echo "a training run is already going; ./train/stop.sh first"; exit 1
fi

t_total() {
  [ -f "$ck" ] || { echo 0; return; }
  $py -c "import torch;print(int(torch.load('$ck',map_location='cpu',weights_only=False)['t_total']))"
}

start=$(t_total)
until=$(awk "BEGIN{print int($start + $hours * 60)}")   # HOURS may be fractional
lineup=2,2,0,0,0,4
[ -f "runs/$run-lineup.txt" ] && lineup=$(cat "runs/$run-lineup.txt")
# extra ppo.py flags for this run, one line (a resume takes them from the
# command line, not the checkpoint), e.g. "--low-penalty 0.02"
extra=""
[ -f "runs/$run-args.txt" ] && extra=$(cat "runs/$run-args.txt")
# flags for the first start only (no checkpoint yet), e.g. "--init-from ..."
# (ppo.py refuses --init-from together with --resume)
first=""
[ -f "runs/$run-init.txt" ] && first=$(cat "runs/$run-init.txt")

(
  export PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True
  echo "$(date '+%F %T') go $hours h: $run from $start to $until min, lineup $lineup $extra" >> $log
  for try in 1 2 3 4 5; do
    t=$(t_total)
    [ "$t" -ge "$until" ] && break
    out=runs/$run-$(date +%Y%m%d-%H%M).out
    echo "$(date '+%F %T')   start at $t min, log $out" >> $log
    $py ppo.py --name $run --minutes $until --compile --snapshot-every 50 \
      --lineup $lineup $extra $([ -f "$ck" ] && echo --resume $ck || echo $first) > $out 2>&1
    rc=$?
    echo "$(date '+%F %T')   exit $rc" >> $log
    grep -q 'no legal action' $out && echo "$(date '+%F %T')   NOTE: 'no legal action' state dump in $out" >> $log
    [ $rc = 0 ] && break
  done
  echo "$(date '+%F %T') done at $(t_total) min" >> $log
) > /dev/null 2>&1 < /dev/null &
disown
echo "training $run for $hours h (until $until min of training); log: train/$log"
