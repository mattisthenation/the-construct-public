# The Construct — Loop Log

Dated narrative of each autonomous build iteration. Newest entries at the top.
`PROGRESS.md` holds the live state/checklist; this file is the history of *why*.

---

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

**Decisions:** Matt chose refactor-in-place + CLAUDE.md-as-truth + fully-autonomous.

**Next:** Rename binary to `construct`; then the high-value narrative work —
the deterministic `remind-me` handler that proves the thesis.
