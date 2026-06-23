#!/usr/bin/env bash
# Build a self-contained release tarball of The Construct for macOS.
#
#   ./scripts/release.sh
#
# Produces dist/theconstruct/ (binary + prompts + starter config + SETUP.md) and
# theconstruct-<arch>-macos.tar.gz at the repo root. Run on the same CPU arch you
# intend to deploy to (the binary is native, not cross-compiled).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# arm64 (Apple Silicon) reports "arm64"; normalize to the conventional "aarch64".
ARCH="$(uname -m)"
[ "$ARCH" = "arm64" ] && ARCH="aarch64"
STAGE="dist/theconstruct"
TARBALL="theconstruct-${ARCH}-macos.tar.gz"

echo "==> Building release binary..."
cargo build --release -p construct-cli

echo "==> Staging $STAGE..."
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp target/release/entertheconstruct "$STAGE/"
cp -R prompts "$STAGE/"

echo "==> Writing SETUP.md..."
cat > "$STAGE/SETUP.md" <<'SETUP'
# The Construct — setup on this machine

A self-contained build of The Construct. `prompts/`, `construct.db`, and `construct.toml`
resolve relative to the **config file's directory**, so the simplest setup is to keep
everything in this folder and either run from here or point the binary at it with
`export CONSTRUCT_HOME="$PWD"`. Then `entertheconstruct` works from any directory.

## 1. Install + start Ollama, pull your models
```sh
brew install ollama        # or download from https://ollama.com
ollama serve               # leave running (defaults to http://localhost:11434)
ollama pull qwen2.5:7b     # whatever models construct.toml references
ollama pull qwen2.5:14b
```

## 2. Run setup
```sh
./entertheconstruct setup
```
The wizard asks for your Obsidian vault path (writes a starter construct.toml if
none exists — an existing one is never overwritten) and offers to store API keys
(e.g. Tavily for the research pipeline) in `.env` next to the config, chmod 600,
loaded automatically at startup. Re-run it anytime to rotate a key.

Prefer hand-editing? `$EDITOR construct.toml` works the same as before: set
`vault.path`, each agent's `model`/`base_url`, and uncomment `[inbox]` /
`[journal]` / `[schedule]` / `[briefs]` to enable automations.

## 3. Validate + run
```sh
./entertheconstruct config-check     # confirms config parses
./entertheconstruct watch            # live dashboard (q quit · p pause · o open today's note); --headless for plain logs
```
Logs print to stderr; use `RUST_LOG=construct=debug ./entertheconstruct watch` for detail.
`construct.db` (run history) is created in this folder. Check activity with
`./entertheconstruct runs`.

## Notes
- The watcher must be running for the `[schedule]` daily summary to fire (it's an
  in-process timer, with catch-up if the machine was asleep — not system cron).
- To put it on PATH: `cp entertheconstruct /usr/local/bin/`. Set `export
  CONSTRUCT_HOME=/abs/path/to/this/folder` (or pass `--config /abs/path/construct.toml`)
  so the DB and prompt files resolve consistently from any directory.
SETUP

echo "==> Packaging $TARBALL..."
tar -C dist -czf "$TARBALL" theconstruct

echo "==> Done."
echo "    Tarball: $ROOT/$TARBALL"
echo "    Staged:  $ROOT/$STAGE"
ls -lh "$TARBALL"
