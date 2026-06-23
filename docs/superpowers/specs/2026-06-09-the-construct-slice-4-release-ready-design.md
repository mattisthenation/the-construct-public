# The Construct — Slice 4: Release-Ready (Briefs, Polish & TUI)

**Date:** 2026-06-09
**Status:** Approved design, pending implementation plan

## Goal

Make The Construct a releasable product someone can install, run, and upgrade
without reading the source — and make the running experience look like a
product, not a log dump. Adds the Daily Briefs integration, fixes carryover
duplication, upgrades the inbox index and daily recap, adds journal tagging,
and ships an interactive setup wizard plus a ratatui dashboard.

## Features

### 1. Interactive setup & install lifecycle

A new `entertheconstruct setup` subcommand (wizard lives in the binary, not
bash, so it works from a downloaded tarball):

- Prompts for vault path (validates the directory exists and looks like an
  Obsidian vault), managed folder, inbox folder, journal folder — each with
  current/default values shown.
- Prompts for API keys (Anthropic, Tavily) with masked input. Keys are written
  to `~/.theconstruct/.env` with mode `0600`. The config keeps the existing
  `api_key_env` model (env-var *names* in `construct.toml`); the binary loads
  `.env` at startup (before config validation) so those vars are populated.
- Idempotent: re-running shows existing values as defaults — safe for key
  rotation or path changes.
- `--non-interactive` flag accepts all values as arguments for scripting and
  tests.
- `scripts/setup-home.sh` becomes a thin bootstrapper (create home dir, deploy
  prompts) that ends by invoking `entertheconstruct setup`.
- `scripts/update.sh` keeps backup/migrate/redeploy behavior and ends with a
  clear "restart with: …" instruction.

### 2. Journal tags

New `tagging` module in `construct-engine` with one idempotent operation
`ensure_journal_tag(note, date)`:

- Adds `journal/YYYY/MM/DD` to frontmatter `tags:` (creating frontmatter if
  absent).
- Appends literal `#journal/YYYY/MM/DD` as the last line of the body if the
  tag is not already present anywhere in the note.
- Applied to: day notes when rendered (tagged with their own date), and any
  note the engine touches — inbox-filed notes, agent-edited notes (tagged with
  the processing date). Backfill happens naturally on touch.
- Uses the existing atomic write path. Tag writes are marked as engine writes
  so the watcher ignores them (same pattern as managed-block writes) — no
  feedback loops.

### 3. Inbox index table

The `inbox-log` managed block in `Inbox/_index.md` becomes a markdown table:

```markdown
| Note | Outcome | Destination | When |
|---|---|---|---|
| [[Reading/some-idea\|some-idea]] | moved | Reading | 2026-06-09 |
```

- The Note column is a wikilink to the note's **new** location, so clicking
  works after the move.
- Same semantics as today: dedupe by note name, update the row in place on
  re-processing.
- Migration: legacy bullet-format lines are converted to table rows the first
  time the block is rewritten.

### 4. Daily Briefs pipeline (event-driven + hash guard)

An external Claude workflow writes Google-Workspace-derived briefs into the
vault at `AI/DailyBriefs/` with a `YYYY-MM-DD` date in each filename.

- Config: optional `[briefs]` section, `folder = "AI/DailyBriefs"`. Section
  absent → feature off.
- Watcher: `classify()` emits `VaultEvent::BriefChanged { path, date }` for
  `.md` files under the briefs folder whose filename contains a parseable
  `YYYY-MM-DD`. Other files in the folder are ignored.
- Hash guard: brief content hash stored in the DB (same pattern as the inbox
  pipeline). Unchanged content → no-op; re-saves don't re-trigger the agent.
- `run_brief()` pipeline:
  1. Deterministic: update a new `daily-brief` managed block in the matching
     day note — wikilink to the brief plus its extracted headings/bullets.
  2. Agentic: re-run the daily summary with the brief content as context.
- Retroactive: a brief landing after the scheduled daily run still updates
  that day's note.

### 5. Carryover dedupe fix

Yesterday's `daily-carryover` block contains `- [ ]` checkboxes, so today's
scrape re-collects them and carryover compounds day over day. Fix:

- `scrape_open_checkboxes` normalizes task text: strip checkbox syntax, trim,
  collapse internal whitespace.
- The merge excludes any task already present (normalized match) in today's
  tasks or carryover blocks.
- A task appearing in both yesterday's tasks and carryover blocks counts once.
- All pure functions with unit tests.

### 6. Robust daily recap

- Prompt extracted from `run_daily_summary()` into `prompts/daily_summary.md`
  with template slots; redeployed by `update.sh` like other prompts, tunable
  without recompiling.
- Inputs grow from filenames to: changed-note **excerpts** (capped per-note
  and in total), completed tasks, carryover delta, and today's brief content.
- Output grows: TL;DR paragraph + highlights list + action items, rendered
  into the `daily-summary` managed block.

### 7. TUI dashboard

`entertheconstruct run` renders a ratatui dashboard; falls back to the plain
log stream when stdout is not a TTY or with `--headless` (launchd/background
use unchanged).

- Layout: header (version, vault path, uptime, watch status), status line
  (next scheduled run, pending-review count), scrolling activity feed, footer
  keybinds (`q` quit, `p` pause watching, `o` open today's note).
- Internals: the orchestrator publishes events on a `tokio::sync::broadcast`
  channel; the TUI is a pure consumer and never blocks pipelines.
- Logs continue to be written to `~/.theconstruct/logs/` in both modes.

## Architecture notes

- Briefs follow the established watcher → `VaultEvent` → orchestrator pipeline
  pattern; no new subsystems beyond the broadcast channel for the TUI.
- All vault writes go through the existing atomic write path; managed blocks
  remain the only engine-owned regions of user notes (plus the additive
  journal tag, which is append/frontmatter-merge only and idempotent).
- Token-spend guards: brief hash guard, recap input caps.

## Error handling

- Setup wizard: validates vault path before writing config; refuses to write
  `.env` without `0600`; never echoes keys.
- Briefs: unparseable filename → skipped with a logged warning, never an
  error loop. Missing day note → created via the normal day-note render path.
- Recap: agent JSON parse failure logs the raw response and leaves the prior
  `daily-summary` block intact (never blanks a section).
- TUI: panics in the UI layer must not take down the engine — UI runs in its
  own task; engine continues headless if the UI task dies.

## Testing

- Unit: tag idempotency (frontmatter + literal, both directions), carryover
  normalization/dedupe, brief filename date parsing, index bullet→table
  migration, recap input capping.
- Integration: brief lands → day note gains `daily-brief` block; brief
  re-saved unchanged → no agent call (hash guard); setup wizard via
  `--non-interactive`.
- Manual: fresh-install walkthrough from tarball (install → setup → run in
  three commands) before release.

## Implementation order

1. Carryover fix + journal tags + index table (small, deterministic)
2. Daily Briefs pipeline
3. Recap upgrade
4. Setup wizard + install/update script polish
5. TUI dashboard
6. Release pass (README for strangers, tarball, version bump)

## Out of scope

- Web dashboard, multi-vault support, Keychain key storage (revisit if the
  `.env` approach proves insufficient), configurable brief filename patterns
  beyond `YYYY-MM-DD` matching.
