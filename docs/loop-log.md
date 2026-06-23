# The Construct — Loop Log

Dated narrative of each autonomous build iteration. Newest entries at the top.
`PROGRESS.md` holds the live state/checklist; this file is the history of *why*.

---

## 2026-06-23 — Iterations 3–6: handlers, providers, audit, TUI

- **run + doctor + naming** (iter 3): `construct run <note>` (shared
  `build_orchestrator` helper) and `construct doctor` (config/vault/provider/key
  preflight). Canonical handler names `remind-me`/`file-this`/`research-this`
  (old names aliased). Background agents produced `examples/sample-vault/` and the
  CI/release/Homebrew/license set.
- **cloud providers** (iter 4): `construct-model-cloud` — OpenAiProvider (reuses
  Ollama's OpenAI-compatible codec + bearer token) and AnthropicProvider
  (`/v1/messages` codec). `provider_for()` factory + `Agent.api_key_env`.
- **audit fix** (iter 5): dropped sqlx `macros`/`migrate` (pulled rsa via unused
  mysql/postgres) → run schema via `raw_sql` over embedded idempotent migrations.
  `cargo audit` exits 0; two transitive ratatui warnings remain (documented).
- **file-this + Priori** (iter 6): explicit `priori::judge` deterministic-vs-escalate
  seam; file-this gets a deterministic keyword-rule tier (zero model on match).
  Dashboard TUI rebuilt to the mockup (two panes + four-box bottom row + cheap
  digital rain) by a background agent.

Concurrency note: the TUI agent briefly `git stash`ed engine WIP to isolate its
tests, which surfaced as transient "reverted" file snapshots mid-build; it
restored everything (`git stash pop`) and the final tree is intact. 217 tests green.

## 2026-06-23 — Iteration 2: the `remind-me` handler (thesis proof)

Built the deterministic `remind-me` pipeline — the handler that proves "most of
your agent calls didn't need to be model calls."

- New `pipelines/remind.rs`: a hand-rolled deterministic parser for
  "remind me to <task> [<when>]". Handles today/tonight/tomorrow, "in N
  days/weeks/months/hours/minutes", weekdays ("next friday"), ISO dates, and
  "at H[:MM][am|pm]" — relative to an injected clock. No NLP/model dependency.
  Writes a `reminder` managed block + `construct_reminder`/`construct_reminder_due`
  frontmatter, marks the note done. 14 unit tests.
- `PipelineKind::RemindMe` + `is_deterministic()`. Orchestrator `run_remind`
  never touches `self.provider`.
- Proof test: orchestrator runs the pipeline against a `PanicModel` that panics
  if `chat()` is ever called — the test passing IS the zero-model-call guarantee.
- `EventKind::Deterministic` ("no-model"); the dashboard renders it bold green and
  the activity log says "done (no model call)". The thesis, made visible.

200 tests pass; clippy/fmt clean.

## 2026-06-23 — Iteration 1: rename + XDG config (see commit)

Binary `entertheconstruct` → `construct`; config moved to XDG
`~/.config/construct/config.toml` with `$CONSTRUCT_HOME` portable override.

## 2026-06-23 — Iteration 0: baseline green

**State found:** Personal Construct's 8 crates copied into the repo, but no root
workspace manifest, so nothing built. No git commits, no PROGRESS.md.

**Did:**
- Reconstructed root `Cargo.toml` (workspace members + `[workspace.dependencies]` +
  `[workspace.package]`), inferring versions from crate API usage. Notable: sqlx needs
  the `macros` feature for `migrate!`; reqwest pinned to `rustls-tls` (static-binary friendly).
- Build green; 184 tests pass; clippy `-D warnings` clean; fmt clean.
- Added `.gitignore` (target, dist, *.db, .env, .DS_Store). `dist/` was a packaged
  build artifact (incl. a compiled binary) — excluded from source.
- Scrubbed the one personal tell: a `/Users/matt/Vault` test fixture → `/Users/example/Vault`.
- Wrote `PROGRESS.md` (decisions, gap analysis, DoD checklist) and this log.

**Decisions:** the maintainer chose refactor-in-place + spec-as-truth + fully-autonomous.

**Next:** Rename binary to `construct`; then the high-value narrative work —
the deterministic `remind-me` handler that proves the thesis.
