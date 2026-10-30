#!/bin/bash
# Stop a training run started by go.sh. The last checkpoint stays (it is
# written at every eval, every few minutes), so ./train/go.sh continues it.
# go.sh shows up as "/bin/bash ./go.sh 8 RUN" or "bash train/go.sh 8 RUN";
# the pattern must not match a shell that merely mentions go.sh in a command
pkill -f '^(/bin/)?bash [^ ]*go\.sh [0-9]' 2>/dev/null
if pkill -INT -f '^\.\./\.venv/bin/python ppo.py'; then
  echo "stopping training"
  echo "$(date '+%F %T') stopped by stop.sh" >> "$(dirname "$0")/runs/go.log"
  # a run in the middle of a rollout can ignore the interrupt for minutes
  # (2026-10-30); give it 30 seconds, then kill it
  for _ in $(seq 30); do
    pgrep -f '^\.\./\.venv/bin/python ppo.py' > /dev/null || exit 0
    sleep 1
  done
  pkill -f '^\.\./\.venv/bin/python ppo.py' && echo "killed it after 30 s"
else
  echo "no training running"
fi
