# The Construct — Build Progress

> Running memory across loop iterations. Trust this over in-context recollection;
> reconcile against actual repo state at the start of each loop. See `docs/loop-log.md`
> for the dated narrative of what happened each iteration.

## Decisions (from the maintainer, 2026-06-23)
- **Refactor the existing personal-Construct crates in place** (don't clean-room rebuild).
- **CLAUDE.md is the source of truth** when it conflicts with existing code (naming, handlers).
- **Run fully autonomous** to the 8-point Definition of Done.

## Current state snapshot (updated each loop)
- Workspace builds (`cargo build` green). Reconstructed the missing root `Cargo.toml`
  workspace manifest (all crates referenced `.workspace = true` but the root was never copied in).
- **184 tests pass**, `cargo fmt --check` clean, `cargo clippy --all-targets -D warnings` clean.
- 8 crates: `construct-{cli,core,config,engine,store,obsidian,tools,model-ollama}`.

## Gap analysis vs CLAUDE.md spec (the work list)
Ordered roughly by value toward Definition of Done.

| # | Item | Status |
|---|------|--------|
| 1 | Workspace builds + green baseline | ✅ done |
| 2 | Rename binary `entertheconstruct` → `construct` | ✅ done |
| 3 | Config path → XDG `~/.config/construct/config.toml` (+ env override) | ✅ done |
| 4 | Add `construct run <note>` subcommand | ✅ done |
| 5 | Add `construct doctor` subcommand | ✅ done |
| 6 | **`remind-me` handler — fully deterministic, zero model calls (thesis proof)** | ✅ done |
| 7 | `file-this` deterministic-first (rules before any model) | ✅ done (keyword rules → folder, zero model; escalates on miss) |
| 8 | Internal module naming: Priori (judge) / Determa (execute) | ✅ done (priori::judge seam; Determa = deterministic pipelines) |
| 9 | Cloud providers: Anthropic + OpenAI-compatible (escalation tier) | ✅ done |
| 10 | TUI: recent-notes pane, matrix-rain panel, logo placeholder | ✅ done |
| 11 | `examples/sample-vault/` with demo notes for all 3 handlers | ✅ done |
| 12 | Homebrew tap + formula, Apple Silicon + Intel builds | ✅ done (formula + release.yml; sha filled at first real release) |
| 13 | GitHub Actions CI: fmt, clippy -D warnings, test, cargo audit, release build | ✅ done |
| 14 | `cargo audit` clean (or findings documented) | ✅ done (0 vulns; 2 transitive warnings to document in security.md) |
| 15 | `docs/security.md` security audit | ⬜ todo |
| 16 | Docs: 30s README, config reference, handler-authoring guide, `docs/design-todo.md` | ⬜ todo |

### Notes that map to spec handlers
- `research-this` ← `pipelines/research.rs` (exists, matches spec — escalates + web tools + grounding gate).
- `file-this` ← `pipelines/{organize,tag,inbox}.rs` (exists but escalate-only; needs deterministic rule tier).
- `remind-me` ← **does not exist**; must build. Highest narrative value.
- Other existing pipelines (brief, daily, journal_tag, summarize) are extra; keep but not in the launch-bar 3.

## Already-good (don't rebuild)
- `ModelProvider` trait in `construct-core` is pluggable; Ollama is the default impl.
- `construct-config` is TOML-driven; vault/model/url/prompts already externalized (path resolution needs XDG fix).
- `gate.rs` does model-output grounding validation (rejects fabricated sources) — keep.
- SSRF guard in `construct-tools/web_fetch.rs` blocks private IP ranges — keep (those "192.168" hits are correct test cases, not personal data).

## Needs maintainer decision (route around, don't block)
- (none yet)

## Deferred / parked (resolved)
- Legacy `scripts/` (old single-folder dist model) were **removed**; distribution is now
  the Homebrew tap + `release.yml`.

## Definition of Done checklist — ALL MET ✅
1. ✅ Tests/clippy/fmt/audit green — 218 tests, `clippy -D warnings` clean, `fmt --check` clean,
   `cargo audit` 0 vulnerabilities (2 transitive ratatui warnings, documented/accepted).
2. ✅ 3 handlers demoable + remind-me zero-model — verified `construct run` on the sample
   vault for remind-me (zero model) and file-this (zero model on keyword match); both proven
   via `PanicModel` tests. research-this pipeline intact (needs Ollama to run live).
3. ✅ Homebrew-installable + doctor passes — `HomebrewFormula/construct.rb` + `release.yml`
   build both mac arches; `doctor` passes on the sample vault. (First real release fills the
   formula sha256 + publishes the tap — a maintainer step, documented in docs/install.md.)
4. ✅ Fully configurable — XDG config, `$CONSTRUCT_HOME`/`--config` overrides; remote Ollama
   or a cloud provider is a config-only change (base_url / provider / api_key_env).
5. ✅ Daemon robust — per-note size cap (4 MiB), caught handler panics, per-note locks,
   bounded timeouts/iterations, reconcile-on-restart; one bad note never wedges the loop.
6. ✅ TUI matches mockup + headless — two panes (Activity + Recent Notes) over four boxes
   (logo · digital rain · commands · status); `--headless` for backgrounding.
7. ✅ Security audit complete — docs/security.md, no unmitigated release-blocker items.
8. ✅ Public-ready docs + clean history — README (30s hook), configuration, handlers,
   security, install, design-todo; conventional-commit history; no personal data
   (CLAUDE.md untracked + gitignored, personal artifacts removed).

---

## LAUNCH-READINESS SUMMARY (loop exit, 2026-06-23)

**The Construct is at the launch bar.** Starting from a non-building pile of copied crates,
the loop delivered a distributable, installable, configurable, updatable tool.

**What shipped**
- A single Rust binary `construct` (workspace of 9 crates) with subcommands
  `setup / init / config-check / doctor / run / watch / status / runs`.
- The deterministic-first thesis made real and *checkable*: **Priori** (judgment seam,
  `priori::judge`) → **Determa** (deterministic pipelines). `remind-me` is fully
  deterministic (proven zero model calls); `file-this` is deterministic-first (keyword
  rules → folder, escalating to a model only on a miss); `research-this` is the escalation
  path with source-grounding.
- LLM-agnostic, local-first: Ollama default + opt-in Anthropic / OpenAI-compatible cloud.
  Zero egress with an Ollama-only config; the cloud boundary is explicit and auditable.
- A read-only TUI dashboard; robust always-on daemon; atomic vault writes; SSRF guard;
  secrets via env-var name only.
- Packaging: Homebrew formula + tag-driven release workflow; CI runs fmt/clippy/test/audit.
- Docs: README, configuration, handlers, security, install, design-todo, sample vault.

**Known follow-ups (non-blocking, for the maintainer)**
- Publish a real `homebrew-the-construct` tap and fill the formula sha256 from the first
  `vX.Y.Z` release (the release workflow prints them).
- Optional hardening parity: extend the `catch_unwind` wrap to the Inbox/Broadcast routes;
  make `MAX_NOTE_BYTES` config-driven.
- Design assets (logo, rain styling, palette) — swap-in points marked; see docs/design-todo.md.
- Live-model validation of the `research-this` prompt against a small local model.

**Loop status: COMPLETE.** Per the build directive, halting rather than inventing new scope.
