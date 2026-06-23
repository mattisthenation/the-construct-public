#!/usr/bin/env bash
# Pull the latest source, rebuild + reinstall the binary, and redeploy prompts.
#
#   ./scripts/update.sh
#
# Leaves construct.toml and construct.db untouched (except a timestamped DB backup).
# sqlx migrations apply automatically, idempotently, on the next launch. Stop the
# watcher (Ctrl-C) before running so the DB copy is consistent (no WAL).
set -euo pipefail

HOME_DIR="${CONSTRUCT_HOME:-$HOME/.theconstruct}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> Stop the watcher first (Ctrl-C) so the DB copy is consistent."

echo "==> Pulling latest..."
git pull --ff-only

if [ -f "$HOME_DIR/construct.db" ]; then
  ts="$(date +%Y%m%d-%H%M%S)"
  cp "$HOME_DIR/construct.db" "$HOME_DIR/construct.db.bak-$ts"
  # Keep only the 5 most recent backups.
  ls -1t "$HOME_DIR"/construct.db.bak-* 2>/dev/null | tail -n +6 | xargs -r rm -f
  echo "==> Backed up DB -> construct.db.bak-$ts"
fi

echo "==> Building + installing binary..."
cargo install --path crates/construct-cli --locked --force   # drop --locked if Cargo.lock lags

echo "==> Deploying prompts..."
mkdir -p "$HOME_DIR/prompts"
cp -R prompts/. "$HOME_DIR/prompts/"

echo "==> Done. Migrations apply automatically on next launch (sqlx, idempotent)."
echo ""
echo "Update complete. Restart The Construct:"
echo "  entertheconstruct watch        (dashboard)"
echo "  entertheconstruct watch --headless   (background/launchd)"
