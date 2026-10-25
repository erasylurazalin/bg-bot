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
# 3,1,1,1,0,2 (3 learner, 1 snapshot, 1 standard, 1 rush, 0 random, 2 real).
set -u
hours=${1:?usage: go.sh HOURS [RUN]}
run=${2:-r1025-attn}
cd "$(dirname "$0")"
log=runs/go.log
ck=runs/$run/ckpt.pt
py=../.venv/bin/python
[ -f "$ck" ] || { echo "no checkpoint $ck"; exit 1; }
if pgrep -f "^$py ppo.py" >/dev/null || pgrep -f "^\.\./\.venv/bin/python ppo.py" >/dev/null; then
  echo "a training run is already going; ./train/stop.sh first"; exit 1
fi

t_total() {
  $py -c "import torch;print(int(torch.load('$ck',map_location='cpu',weights_only=False)['t_total']))"
}

start=$(t_total)
until=$(( start + hours * 60 ))
lineup=3,1,1,1,0,2
[ -f "runs/$run-lineup.txt" ] && lineup=$(cat "runs/$run-lineup.txt")

(
  export PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True
  echo "$(date '+%F %T') go $hours h: $run from $start to $until min, lineup $lineup" >> $log
  for try in 1 2 3 4 5; do
    t=$(t_total)
    [ "$t" -ge "$until" ] && break
    out=runs/$run-$(date +%Y%m%d-%H%M).out
    echo "$(date '+%F %T')   start at $t min, log $out" >> $log
    $py ppo.py --name $run --minutes $until --compile --snapshot-every 50 \
      --lineup $lineup --resume $ck > $out 2>&1
    rc=$?
    echo "$(date '+%F %T')   exit $rc" >> $log
    grep -q 'no legal action' $out && echo "$(date '+%F %T')   NOTE: 'no legal action' state dump in $out" >> $log
    [ $rc = 0 ] && break
  done
  echo "$(date '+%F %T') done at $(t_total) min" >> $log
) > /dev/null 2>&1 < /dev/null &
disown
echo "training $run for $hours h (until $until min of training); log: train/$log"
