#!/usr/bin/env bash
# One-time, per-machine setup of the Construct home directory.
#
#   ./scripts/setup-home.sh
#
# Creates ~/.theconstruct/ (override with $CONSTRUCT_HOME), deploys the versioned
# prompts/, and migrates any existing in-repo construct.toml / construct.db into the
# home dir. Never clobbers data already present in the home dir. Run once per machine.
set -euo pipefail

HOME_DIR="${CONSTRUCT_HOME:-$HOME/.theconstruct}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

mkdir -p "$HOME_DIR/prompts"
cp -R "$ROOT/prompts/." "$HOME_DIR/prompts/"        # prompts: versioned source of truth

# Migrate existing live data from the repo dir if present and not already moved.
[ -f "$ROOT/construct.toml" ] && [ ! -f "$HOME_DIR/construct.toml" ] && cp "$ROOT/construct.toml" "$HOME_DIR/"
[ -f "$ROOT/construct.db" ]   && [ ! -f "$HOME_DIR/construct.db" ]   && cp "$ROOT/construct.db"   "$HOME_DIR/"
chmod 600 "$HOME_DIR/construct.toml" "$HOME_DIR/construct.db" 2>/dev/null || true

echo "Construct home ready at $HOME_DIR"
echo
echo "Machine 1: any existing run history was copied here. You can delete the repo's"
echo "  construct.db / construct.toml once you've confirmed the watcher uses the new home."

if command -v entertheconstruct >/dev/null 2>&1; then
  exec entertheconstruct setup
else
  echo "Now run: entertheconstruct setup"
fi
