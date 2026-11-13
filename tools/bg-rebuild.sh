#!/bin/bash
# Build bgpy from a bg-bot commit and install it into ~/projects/bg's venv, so the
# tracker's odds follow the committed engine. The post-commit hook runs it in the
# background when a commit touches crates/; run it by hand with an optional commit
# (default HEAD). Only committed code is built: the commit is exported with
# git archive, so uncommitted work in the tree never reaches bg.
# A running tracker keeps the copy it loaded until it restarts. Log: target/bg-rebuild.log
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BG_PY="${BG_PY:-$HOME/projects/bg/.venv/bin/python}"
REV="$(git -C "$ROOT" rev-parse "${1:-HEAD}")"
SRC="$ROOT/target/bg-rebuild/src"
WHEELS="$ROOT/target/bg-rebuild/wheels"
LOCK="$ROOT/target/bg-rebuild/lock"
mkdir -p "$ROOT/target/bg-rebuild"
exec 9>"$LOCK"
flock 9
echo "$(date '+%F %T') build ${REV:0:7}"
rm -rf "$SRC" "$WHEELS"; mkdir -p "$SRC" "$WHEELS"
git -C "$ROOT" archive "$REV" | tar -x -C "$SRC"
CARGO_TARGET_DIR="$ROOT/target/bg-rebuild/cargo" \
  "$ROOT/.venv/bin/maturin" build --release -m "$SRC/crates/bgpy/Cargo.toml" \
  -i "$BG_PY" -o "$WHEELS" --quiet
"$BG_PY" -m pip install --quiet --disable-pip-version-check --force-reinstall --no-deps "$WHEELS"/bgpy-*.whl
echo "$REV" > "$ROOT/target/bg-rebuild/installed"
echo "$(date '+%F %T') installed ${REV:0:7} into $BG_PY"
