# Next-session handoff — The Construct, Slice 3 (Automations)

> Paste everything below the line into a fresh Claude Code CLI session started in
> `~/Sites/theconstruct`. Self-contained. The design is DONE and approved; your job is
> writing-plans → build, not re-designing.

---

You are implementing **Slice 3 (Automations)** of The Construct, a local-first Rust
agent runtime that watches an Obsidian vault and runs local-model (Ollama) note
actions. Guiding principle: **deterministic first** — a deterministic outer shell owns
all triggers and side effects; a single agentic step does only open-ended reasoning and
returns JSON only (never touches files).

## Read first
1. **The approved design spec:**
   `docs/superpowers/specs/2026-06-02-the-construct-slice-3-automations-design.md`
   (this is the source of truth — all design decisions are settled in it).
2. Prior context: `docs/superpowers/specs/` (Slice 1 + Slice 2 designs), and the project
   memory.

## Repo state
Slice 1 (research) and Slice 2 (Summarize/Tag/Organize note actions on a multi-pipeline
dispatch engine) are **shipped and merged to `main`**. Binary: `entertheconstruct`
(crate `construct-cli`). Confirm you're on `main` and green before starting:
```
git checkout main && git pull --ff-only
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --all -- --check
```
(If the Slice 3 spec file isn't present on `main`, it's at commit `d7d6c12` /
`46124ee` on local main, or on branch `slice-2-note-actions` — cherry-pick or pull it.)

## What to build (full detail in the spec — summary only here)
Three independent subsystems → **three separate implementation plans**, each shipping
working software, each built with TDD + verified against git:

- **Plan 1 — Trigger generalization (enabler) + config + stats docs.** Generalize the
  engine from vault-events-only to a `Trigger` abstraction feeding the same
  `Orchestrator` (behavior-preserving for existing tag/status triggers — like Slice 2's
  Phase 0 did for pipelines). Adds `IdleTrigger` (polls Inbox by file mtime) and
  `ScheduleTrigger` (in-process 1am timer **with catch-up** via a last-run row in
  construct.db). New `PipelineKind::Inbox` + `DailySummary`. New `[inbox]`/`[journal]`/
  `[schedule]` config tables (features OFF unless present). Shared loop-guard exclusion
  (`_index`, `journal/` tree, scope limits). Feature C = **docs only**: document the
  construct.db schema (`runs`, `run_events`) + example SSH/sqlite3 queries for a future
  monitoring app (NO app, NO server built).

- **Plan 2 — Inbox auto-processing.** `IdleTrigger` watches **top-level files in the
  Inbox folder ONLY** (not recursive, not the rest of the vault). When a note is idle
  ≥ idle_minutes (default 30, by file mtime): (1) fetch+summarize up to the first 5
  URLs, skip-on-fail; (2) summarize (reuse `apply_summary`); (3) tag (reuse
  `apply_tags`); (4) move decision — auto-move ONLY if the model's destination exactly
  matches an existing scanned folder (→ status done); otherwise leave in Inbox, write
  the recommended destination at the top (managed block), status review. The
  recommendation MAY name a not-yet-existing folder (suggestion only) — Construct never
  creates a folder or auto-moves into a non-existing one. NOTE: this is looser than
  Slice 2's `validate_organize` (which rejects unknown folders) — branch on "is this an
  existing folder?" rather than failing the gate. (5) maintain `Inbox/_index.md`
  (managed block) logging each note's outcome.
  **No-reprocess-loop guarantee (critical):** the idle scan SKIPS any note that already
  has a `construct_status` field (plus `_index`/managed files). So an unmoved note ends
  at status `review`, sits in Inbox, and never re-triggers despite aging mtime; to
  re-run, the user clears its `construct_status`.

- **Plan 3 — Daily summary (scheduled).** `ScheduleTrigger` fires at `daily_time`
  (default 01:00; catch-up if missed). Scan vault for notes with mtime in the previous
  calendar day (manual + Construct edits; excludes journal/ and managed files). Ensure
  `journal/YYYY/MM/` exists (year folder effectively only Jan 1), create day note
  `DD.md` (zero-padded) if absent. Write four managed sections, idempotent
  (update-in-place on re-run): **Today's Task List** = open `- [ ]` checkboxes scraped
  deterministically from YESTERDAY'S NOTES + carryover only (de-duped); **Carryover
  from yesterday** = still-unchecked `- [ ]` items from yesterday's journal day-note;
  **Yesterday summary** = LLM-written prose recap (Librarian agent, JSON, gated) of
  yesterday's changed notes; **Other notes** = links to yesterday's changed notes not
  otherwise captured. Task list/carryover are deterministic (local models invent tasks);
  only the prose recap is agentic. All times are local time.

## Reusable building blocks (REUSE — do not reimplement)
`apply_summary`, `apply_tags`/`validate_tags`/`merge_tags`, organize gate +
propose/accept/reject + move-with-collision, `construct_obsidian::vault::list_folders`/
`existing_tags`/`existing_tags_excluding`, managed blocks `upsert_named`/
`upsert_named_at_top`/`remove_named`, web fetch tool (`construct-tools`), per-note
async-mutex serialization, the `Store` trait over construct.db, the `construct_status`
frontmatter lifecycle, reconcile-on-startup.

## Carried-over backlog (fold in opportunistically)
- `.gitignore` root `construct.toml` (untracked; real vault path + live-looking Tavily
  key). **Safe quick win — just do it.**
- Chat TUI async/streaming send + spinner (currently blocking with a static
  "thinking…" hint).
- `extract_json` JSON-repair/retry for flaky local-model output.
- Slice 1 item 3b: configurable agent-loop iteration budget + force a final-answer
  attempt on the last iteration instead of hard-failing at the cap (hardcoded 8 in
  watch_loop).

## Process (these mattered in Slices 1 & 2 — honor them)
1. Use **superpowers:writing-plans** to turn the spec into THREE phased, bite-sized,
   TDD plans under `docs/superpowers/plans/` (one per subsystem). Then build each with
   **superpowers:subagent-driven-development**.
2. **Verify every task against git** — real `git rev-parse HEAD`, `git show --stat`,
   `cargo test` counts. NEVER trust subagent self-reports. NEVER reference a commit SHA
   you have not read from git THIS turn (hallucinated SHAs caused cascading rework
   before).
3. **Do not batch a SHA-dependent `Bash` call in the same parallel tool block as an
   `Agent` dispatch** — a git error cancels the whole batch and silently drops the
   dispatches. Run verification, read results, THEN dispatch.
4. **Don't re-read whole files into tool output** — use `git show --stat` + targeted
   `Read`/`sed` ranges to conserve context.
5. **TDD each task** (failing test → run/confirm fail → minimal impl → run/confirm pass
   → commit). Commit author `Matt <matt@matthewlittlehale.com>` with trailer
   `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
6. Work on a feature branch; open a PR per subsystem (or one for the slice). **Do NOT
   push `main` directly** (the environment guard blocks it anyway).
7. Keep `cargo test` / `clippy --all-targets -- -D warnings` / `fmt --all -- --check`
   green at every commit.

## Suggested order
0. Quick win: `.gitignore` root `construct.toml`.
1. Plan 1 (trigger refactor + config + stats docs) — must be behavior-preserving for
   existing pipelines.
2. Plan 2 (Inbox).
3. Plan 3 (Daily summary).
Each: writing-plans → subagent-driven-development → verify against git.
