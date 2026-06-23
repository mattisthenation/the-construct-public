# The Construct — Slice 3: Automations (Inbox, Daily Summary, Stats Access) — Design Spec

**Status:** Approved design (2026-06-02). Implementation NOT started — to be built in a
separate (Claude Code CLI) long-running session via writing-plans →
subagent-driven-development.

**Goal:** Add time/idle-driven automations on top of Slice 2's tag-driven actions:
(A) an Inbox folder that auto-enriches/summarizes/tags/files notes after they go idle,
(B) a scheduled daily-summary journal entry, and (C) documented access to The
Construct's activity/stats data for a future separate monitoring app. Enabled by
generalizing the engine's trigger sources (analogous to how Slice 2 generalized
pipeline dispatch).

## Background / current architecture (as of `main`, Slice 2 shipped)

- Triggers today are **vault events only**: `construct_obsidian::watcher::VaultEvent`
  = `NoteTagged { path, tag }` | `StatusChanged { path, status }`. The watch loop
  (`crates/construct-cli/src/tui/watch_loop.rs`) builds one `Orchestrator` per config
  rule keyed by `match_tag` and routes events to it.
- **Pipelines** in `crates/construct-engine/src/pipelines/` with a `PipelineKind` enum
  (Research, Summarize, Tag, Organize). `Orchestrator::handle_tagged` dispatches by
  `self.pipeline`. Each pipeline = deterministic transforms + a deterministic gate
  (`gate.rs`); the agent returns JSON only and never touches files directly.
- **Reusable building blocks (reuse these; do not reimplement):** `apply_summary`
  (TL;DR managed block at top), `apply_tags` + `validate_tags` (normalize/cap/dedupe) +
  `merge_tags`, organize `validate_organize` (destination must match a real scanned
  folder) + propose/accept/reject + move-on-accept with filename-collision handling,
  vault scan `construct_obsidian::vault::list_folders` / `existing_tags` /
  `existing_tags_excluding`, managed blocks `upsert_named` / `upsert_named_at_top` /
  `remove_named`, web fetch/search tools (`construct-tools`), per-note async-mutex
  serialization, the `Store` trait over `construct.db` (SQLite) with `runs` +
  `run_events`, `construct_status` frontmatter lifecycle
  (queued/running/researching/review/accepted/rejected/done/error), reconcile on
  startup.

## Decomposition (three independent plans)

Per scope rules these are largely independent subsystems. Build as **three separate
plans**, each producing working, testable software:

1. **Plan 1 — Trigger generalization + config + stats docs (the enabler).**
2. **Plan 2 — Inbox auto-processing pipeline (Feature A).**
3. **Plan 3 — Daily summary scheduled pipeline (Feature B).**

Feature C (stats access) is documentation-only and lands inside Plan 1.

---

## Plan 1 — Trigger generalization (enabling refactor)

Generalize the engine from "vault events only" to a `Trigger` abstraction feeding the
same `Orchestrator`. **Behavior-preserving** for all existing pipelines.

**Trigger sources:**
- `TagTrigger` / `StatusTrigger` — today's `NoteTagged` / `StatusChanged` behavior,
  refactored under the new abstraction with no behavior change.
- `IdleTrigger` — polls the configured Inbox folder on an interval; emits a process
  event for a note whose **file mtime is older than `idle_minutes`** (default 30).
- `ScheduleTrigger` — in-process wall-clock timer that fires a scheduled job at the
  configured time (default `01:00`), **with catch-up**: on startup, read the last
  successful run timestamp for the scheduled job from `construct.db`; if the scheduled
  time has passed since then (e.g. laptop asleep at 1am), run it once on launch.

**New pipeline kinds:** extend `PipelineKind` with `Inbox` and `DailySummary`. Both
reuse existing transforms/gates.

**Clock injection:** idle detection and the scheduler take an injectable clock (a trait
or a `now()` fn parameter) so tests can set mtimes / simulate missed schedules without
sleeping. Catch-up is tested by seeding a stale last-run row.

**Config (new TOML tables, mirroring `[actions.*]`; each feature OFF unless its table
is present):**
```toml
[inbox]
folder = "Inbox"          # vault-relative; top-level only
idle_minutes = 30

[journal]
folder = "journal"

[schedule]
daily_time = "01:00"      # local time, HH:MM 24h
```
Extend `Config::validate()` for these (e.g. valid `HH:MM`, positive idle_minutes).
Folder names are configurable; defaults shown.

**Loop guard (shared mechanism — critical):** define a single exclusion check used by
ALL triggers so Construct-managed files never trigger processing:
- Exclude by path/name: the `_index` note, the entire `journal/` tree, and any file
  outside the relevant trigger's scope.
- `IdleTrigger` additionally scopes to **top-level files in the Inbox folder only**
  (NOT recursive, NOT the rest of the vault).

**Feature C — stats access (documentation only, no runtime code):** Add a docs section
(in this spec's appendix and/or `README`) documenting the `construct.db` schema
(`runs`: id, rule, agent, note_path, status, error, created_at, updated_at;
`run_events`: run_id, stage, event, payload, created_at) with example read-only queries
for a future separate monitoring app or `ssh + sqlite3` use:
- recent activity feed (latest run_events joined to runs),
- runs grouped by status / pipeline / day,
- currently-in-review notes.
Backlog (out of scope now): a read-only `entertheconstruct stats` subcommand.

---

## Plan 2 — Inbox auto-processing (Feature A)

`IdleTrigger` watches **top-level files in the configured `Inbox/` folder only**. When a
note is idle ≥ `idle_minutes`, an `Inbox`-pipeline run executes these steps in order
(each gated; any failure routes to `construct_status: error` per existing pattern):

1. **URL enrich** — extract up to the **first 5 URLs** in the note; for each, fetch via
   the existing web-fetch tool and summarize its content into the note;
   **skip-on-fail** (a dead/failing URL is skipped, processing continues). Rationale:
   notes are quick jottings, rarely many links.
2. **Summarize** — reuse `apply_summary` (TL;DR managed block at top).
3. **Tag** — reuse `apply_tags` (prefers existing vault tags).
4. **Move decision** — ask the model for the best destination. If its destination
   **exactly matches a real existing scanned folder**, auto-move the file there (reuse
   organize's move-with-collision logic), setting `construct_status: done`. Otherwise,
   **leave the note in Inbox**, write the **recommended destination at the top** of the
   note (managed block), and set `construct_status: review`. The recommendation MAY name
   a folder that does not exist yet (a suggestion for the human to create) — The
   Construct never creates a folder and never auto-moves into a non-existing one; it
   only ever auto-moves into folders that already exist. (Note: this is slightly looser
   than Slice 2's `validate_organize`, which rejects unknown destinations outright —
   here an unknown/new destination is allowed as a *recommendation only*. The
   implementation should branch on "destination is an existing folder?" rather than
   failing the gate on a new-folder suggestion.)
5. **`_index` update** — maintain `Inbox/_index.md` (a managed block) logging each
   processed note's outcome (e.g. enriched N urls / summarized / tagged /
   moved→`<folder>` or recommended→`<folder>`). `_index` is loop-guarded.

**No-reprocess-loop guarantee (explicitly required):** the idle scan **skips any note
that already has a `construct_status` field** (queued/running/review/done/error/etc.),
plus `_index` and managed files. Therefore:
- A processed-but-UNMOVED note ends at `construct_status: review`, sits quietly in
  Inbox, and is **never re-triggered** despite its aging mtime.
- A moved note leaves Inbox entirely.
- To force a re-run, the user clears the note's `construct_status` field; a brand-new
  note has none and triggers normally.
This reuses the existing `construct_status` lifecycle and the orchestrator idempotency
guard — consistent with how research/organize already gate re-triggers.

---

## Plan 3 — Daily summary (Feature B, scheduled)

`ScheduleTrigger` fires the `DailySummary` pipeline at `schedule.daily_time` (default
01:00; catch-up if missed). It:

1. **Scan** the vault for notes with **filesystem mtime in the previous calendar day**
   (captures both manual and Construct edits). Excludes the `journal/` tree and managed
   files.
2. **Ensure journal path** `journal/YYYY/MM/` exists (create year folder — effectively
   only on Jan 1 — and month folder as needed), and create the day note `DD.md`
   (zero-padded, e.g. `01.md`) if absent.
3. **Write the day note** with four sections:
   - **Today's Task List** — open `- [ ]` checkboxes scraped **deterministically** from
     yesterday's changed notes, plus carryover (de-duplicated).
   - **Carryover from yesterday** — still-unchecked `- [ ]` items pulled
     **deterministically from yesterday's journal day-note** (`journal/<yest
     Y/M/D>.md`), if it exists.
   - **Yesterday summary** — an **LLM-written** prose recap (Librarian agent, returns
     JSON, validated by a gate) of yesterday's changed notes.
   - **Other notes** — links to yesterday's changed notes not otherwise represented.

Mix: agentic prose recap; deterministic task/carryover extraction (local models are
unreliable at inventing task lists). The journal tree is loop-guarded so generated day
notes never trigger Inbox/tag/organize. Idempotent: re-running for a day updates the
day note's managed sections in place rather than duplicating.

---

## Data flow (unchanged shape)

`Trigger source → event → Orchestrator → claim (status, run id, store run) → pipeline
(deterministic transforms + deterministic gate; agent returns JSON only) → write/move →
persist run + run_events`. Existing research/summarize/tag/organize flows are
unaffected by the refactor.

## Error handling

All new pipelines reuse `fail()` → `construct_status: error` + an `error` run event;
the watcher/poller/scheduler survive individual failures. URL-fetch failures in Inbox
step 1 are skipped (not fatal). Missing yesterday journal note → empty Carryover
section, not an error.

## Testing strategy

TDD throughout, pure transforms unit-tested with temp dirs + an injectable clock:
- Idle detection (set file mtime in the past; confirm only top-level Inbox files
  trigger; confirm notes with an existing `construct_status` are skipped — the
  no-loop guarantee).
- URL extraction (0/1/5/>5 URLs; first-5 cap; skip-on-fail path).
- Journal path math incl. month and **year rollover**; day-note creation; zero-padded
  `DD`.
- Checkbox scraping + carryover from a prior day note; de-dup against today's task list.
- Scheduler catch-up: seed a stale last-run row, confirm one run fires on startup;
  confirm no double-fire when up to date.
- Loop guard: `_index` and `journal/` never trigger.
- Behavior-preservation: existing research/tag/etc. trigger tests still pass after the
  refactor.
- Config validation: valid/invalid `daily_time`, idle_minutes; features off when table
  absent.

## Scope / YAGNI

- C builds NO app and NO server — schema docs only.
- No system cron/launchd — scheduler is in-process with catch-up.
- No fuzzy move-confidence — exact existing-folder match only (reuses organize gate).
- No vector DB / frontier models / Notion-Drive (still deferred from earlier slices).

## Carried-over backlog (fold in opportunistically or defer)

- `.gitignore` the root `construct.toml` (untracked; real vault path + live-looking
  Tavily key — accidental-commit risk). **Safe quick win.**
- Chat TUI async/streaming send + spinner (currently blocking with static "thinking…").
- `extract_json` JSON-repair/retry for flaky local-model output (gate currently
  hard-rejects malformed JSON → status=error).
- Slice 1 item 3b: configurable agent-loop iteration budget + force a final-answer
  attempt on the last iteration instead of hard-failing at the cap.
