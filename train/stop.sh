#!/bin/bash
# Stop a training run started by go.sh. The last checkpoint stays (it is
# written at every eval, every few minutes), so ./train/go.sh continues it.
pkill -f '^bash .*train/go.sh' 2>/dev/null
pkill -f '^/bin/bash .*train/go.sh' 2>/dev/null
if pkill -INT -f '^\.\./\.venv/bin/python ppo.py'; then
  echo "stopping training"
  echo "$(date '+%F %T') stopped by stop.sh" >> "$(dirname "$0")/runs/go.log"
else
  echo "no training running"
fi
