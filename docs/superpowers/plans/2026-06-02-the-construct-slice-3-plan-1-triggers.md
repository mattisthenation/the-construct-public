# Slice 3 — Plan 1: Trigger Generalization + Config + Stats Docs

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generalize the engine from "vault events only" to a `TriggerEvent` abstraction, add an injectable `Clock`, a shared loop-guard, schedule catch-up state in the store, two unit-tested trigger cores (idle + schedule), new `PipelineKind` variants, the `[inbox]`/`[journal]`/`[schedule]` config tables, and Feature C stats docs — all behavior-preserving for existing tag/status pipelines.

**Architecture:** This is the enabling refactor. It builds *library* pieces (clock, guard, trigger cores, config, store state) with full unit tests, plus a behavior-preserving `VaultEvent → TriggerEvent` boundary in the watch loop. The idle/schedule trigger *sources* are NOT yet wired into the running watch loop — that wiring lands in Plans 2 and 3 alongside the pipelines that consume them. Existing research/summarize/tag/organize flows must behave identically.

**Tech Stack:** Rust (workspace of `construct-*` crates), tokio, sqlx (SQLite), chrono (already a workspace dep), notify. Commit author `Matt <matt@matthewlittlehale.com>` with trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

**Conventions for every task:** TDD — failing test first, run to confirm fail, minimal impl, run to confirm pass, then commit. Keep `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --all -- --check` green at every commit.

---

## File Structure

- `crates/construct-core/src/clock.rs` — **NEW**: `Clock` trait + `SystemClock`. Re-exported from `lib.rs`.
- `crates/construct-core/src/store.rs` — **MODIFY**: add `get_last_run` / `set_last_run` to the `Store` trait.
- `crates/construct-store/migrations/0002_schedule_state.sql` — **NEW**: `schedule_state` table.
- `crates/construct-store/src/lib.rs` — **MODIFY**: implement the two new `Store` methods.
- `crates/construct-config/src/lib.rs` — **MODIFY**: `InboxCfg`, `JournalCfg`, `ScheduleCfg` (all `Option`), validation.
- `crates/construct-engine/src/guard.rs` — **NEW**: `is_excluded` shared loop-guard.
- `crates/construct-engine/src/triggers/mod.rs` — **NEW**: `TriggerEvent` enum + `From<VaultEvent>`.
- `crates/construct-engine/src/triggers/idle.rs` — **NEW**: `should_process_inbox_note` pure core.
- `crates/construct-engine/src/triggers/schedule.rs` — **NEW**: `due` pure core.
- `crates/construct-engine/src/lib.rs` — **MODIFY**: `pub mod guard; pub mod triggers;`.
- `crates/construct-engine/src/pipelines/mod.rs` — **MODIFY**: add `Inbox` + `DailySummary` to `PipelineKind`.
- `crates/construct-engine/src/orchestrator.rs` — **MODIFY**: stub match arms for the two new kinds.
- `crates/construct-cli/src/tui/watch_loop.rs` — **MODIFY**: route through `TriggerEvent` (behavior-preserving).
- `README.md` (or new `docs/stats-access.md`) — **NEW/MODIFY**: Feature C schema + query docs.

---

### Task 1: `Clock` trait in construct-core

**Files:**
- Create: `crates/construct-core/src/clock.rs`
- Modify: `crates/construct-core/src/lib.rs`

- [ ] **Step 1: Write the failing test** — append to `crates/construct-core/src/clock.rs`:

```rust
//! Injectable wall-clock so idle/schedule logic is testable without sleeping.
use chrono::{DateTime, Local};

/// A source of the current local time. Real code uses `SystemClock`; tests use a fixed clock.
pub trait Clock: Send + Sync {
    fn now_local(&self) -> DateTime<Local>;
}

/// Production clock backed by the OS.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_local(&self) -> DateTime<Local> {
        Local::now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// A clock pinned to a fixed instant, for deterministic tests.
    struct FixedClock(DateTime<Local>);
    impl Clock for FixedClock {
        fn now_local(&self) -> DateTime<Local> {
            self.0
        }
    }

    #[test]
    fn fixed_clock_returns_its_instant() {
        let t = Local.with_ymd_and_hms(2026, 6, 2, 13, 30, 0).unwrap();
        let c = FixedClock(t);
        assert_eq!(c.now_local(), t);
    }

    #[test]
    fn system_clock_is_monotonic_ish() {
        let a = SystemClock.now_local();
        let b = SystemClock.now_local();
        assert!(b >= a);
    }
}
```

- [ ] **Step 2: Wire the module** — add to `crates/construct-core/src/lib.rs` after `pub mod store;`:

```rust
pub mod clock;
```

- [ ] **Step 3: Confirm chrono is a dependency** of `construct-core` (it already is — `crates/construct-core/Cargo.toml` lists `chrono`). If missing, run `cargo add chrono --features serde -p construct-core`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p construct-core clock`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/construct-core/src/clock.rs crates/construct-core/src/lib.rs
git commit -m "feat(core): injectable Clock trait + SystemClock"
```

---

### Task 2: Schedule catch-up state in the store

**Files:**
- Create: `crates/construct-store/migrations/0002_schedule_state.sql`
- Modify: `crates/construct-core/src/store.rs`, `crates/construct-store/src/lib.rs`

The `ScheduleTrigger` (Plan 3) needs to remember the last time a named scheduled job ran, so it can catch up after the laptop was asleep. Store it as `(job, last_run)` where `last_run` is an RFC3339 string.

- [ ] **Step 1: Write the migration** — `crates/construct-store/migrations/0002_schedule_state.sql`:

```sql
CREATE TABLE IF NOT EXISTS schedule_state (
    job       TEXT PRIMARY KEY,
    last_run  TEXT NOT NULL
);
```

- [ ] **Step 2: Write the failing test** — add to the `#[cfg(test)] mod tests` block in `crates/construct-store/src/lib.rs` (create the test module if absent, mirroring existing tests that use `SqliteStore::connect("sqlite::memory:")`):

```rust
#[tokio::test]
async fn schedule_state_roundtrips() {
    let store = SqliteStore::connect("sqlite::memory:").await.unwrap();
    // Absent job → None.
    assert_eq!(store.get_last_run("daily_summary").await.unwrap(), None);
    // Set then read back.
    store.set_last_run("daily_summary", "2026-06-01T01:00:00+00:00").await.unwrap();
    assert_eq!(
        store.get_last_run("daily_summary").await.unwrap().as_deref(),
        Some("2026-06-01T01:00:00+00:00")
    );
    // Upsert overwrites.
    store.set_last_run("daily_summary", "2026-06-02T01:00:00+00:00").await.unwrap();
    assert_eq!(
        store.get_last_run("daily_summary").await.unwrap().as_deref(),
        Some("2026-06-02T01:00:00+00:00")
    );
}
```

- [ ] **Step 3: Run to confirm it fails**

Run: `cargo test -p construct-store schedule_state_roundtrips`
Expected: FAIL (method `get_last_run` not found).

- [ ] **Step 4: Add the trait methods** — in `crates/construct-core/src/store.rs`, inside `pub trait Store`, after `runs_with_status`:

```rust
    /// Read the last-run timestamp (RFC3339 string) for a named scheduled job, if any.
    async fn get_last_run(&self, job: &str) -> Result<Option<String>, StoreError>;
    /// Upsert the last-run timestamp (RFC3339 string) for a named scheduled job.
    async fn set_last_run(&self, job: &str, ts: &str) -> Result<(), StoreError>;
```

- [ ] **Step 5: Implement on SqliteStore** — in `crates/construct-store/src/lib.rs`, inside `impl Store for SqliteStore`:

```rust
    async fn get_last_run(&self, job: &str) -> Result<Option<String>, StoreError> {
        let row = sqlx::query("SELECT last_run FROM schedule_state WHERE job = ?")
            .bind(job)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(row.map(|r| r.get::<String, _>("last_run")))
    }

    async fn set_last_run(&self, job: &str, ts: &str) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO schedule_state (job, last_run) VALUES (?, ?)
             ON CONFLICT(job) DO UPDATE SET last_run = excluded.last_run",
        )
        .bind(job)
        .bind(ts)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(())
    }
```

- [ ] **Step 6: Run the test**

Run: `cargo test -p construct-store schedule_state_roundtrips`
Expected: PASS. Then `cargo test -p construct-store` (all store tests still green; migration runs cleanly).

- [ ] **Step 7: Commit**

```bash
git add crates/construct-store/migrations/0002_schedule_state.sql crates/construct-core/src/store.rs crates/construct-store/src/lib.rs
git commit -m "feat(store): schedule_state table + get/set_last_run for catch-up"
```

---

### Task 3: New config tables `[inbox]` / `[journal]` / `[schedule]`

**Files:**
- Modify: `crates/construct-config/src/lib.rs`

Each feature is OFF unless its table is present, so each is an `Option<...>` field with `#[serde(default)]`. `validate()` checks `daily_time` is `HH:MM` 24h and `idle_minutes > 0`.

- [ ] **Step 1: Write the failing tests** — add to the `tests` module in `crates/construct-config/src/lib.rs`:

```rust
    #[test]
    fn features_off_when_tables_absent() {
        let cfg: Config = toml::from_str(sample()).unwrap();
        assert!(cfg.inbox.is_none());
        assert!(cfg.journal.is_none());
        assert!(cfg.schedule.is_none());
        cfg.validate().unwrap();
    }

    #[test]
    fn parses_inbox_journal_schedule() {
        let toml = format!(
            "{}\n[inbox]\nfolder = \"Inbox\"\nidle_minutes = 45\n\n[journal]\nfolder = \"journal\"\n\n[schedule]\ndaily_time = \"01:00\"\n",
            sample()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let inbox = cfg.inbox.unwrap();
        assert_eq!(inbox.folder, "Inbox");
        assert_eq!(inbox.idle_minutes, 45);
        assert_eq!(cfg.journal.unwrap().folder, "journal");
        assert_eq!(cfg.schedule.unwrap().daily_time, "01:00");
    }

    #[test]
    fn inbox_defaults_folder_and_idle() {
        let toml = format!("{}\n[inbox]\n", sample());
        let cfg: Config = toml::from_str(&toml).unwrap();
        let inbox = cfg.inbox.unwrap();
        assert_eq!(inbox.folder, "Inbox");
        assert_eq!(inbox.idle_minutes, 30);
    }

    #[test]
    fn rejects_zero_idle_minutes() {
        let toml = format!("{}\n[inbox]\nidle_minutes = 0\n", sample());
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
    }

    #[test]
    fn rejects_bad_daily_time() {
        for bad in ["1:00", "25:00", "01:60", "noon", "0100"] {
            let toml = format!("{}\n[schedule]\ndaily_time = \"{}\"\n", sample(), bad);
            let cfg: Config = toml::from_str(&toml).unwrap();
            assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))), "should reject {bad}");
        }
    }

    #[test]
    fn accepts_good_daily_times() {
        for ok in ["00:00", "01:00", "23:59", "9:05"] {
            let toml = format!("{}\n[schedule]\ndaily_time = \"{}\"\n", sample(), ok);
            let cfg: Config = toml::from_str(&toml).unwrap();
            assert!(cfg.validate().is_ok(), "should accept {ok}");
        }
    }
```

> Note: `"9:05"` (single-digit hour) is accepted; `"0100"` (no colon) and `"1:00"`... wait — `"1:00"` is single-digit hour with colon → accepted, so do NOT put `"1:00"` in the reject list. Use the reject list exactly as written above (`"0100"` has no colon; `"1:00"` is NOT in the reject list). Validation rule: split on `:` into exactly 2 parts, parse as integers, `hour ∈ 0..=23`, `minute ∈ 0..=59`, minute part must be exactly 2 digits.

- [ ] **Step 2: Add the config structs** — in `crates/construct-config/src/lib.rs`, add fields to `Config`:

```rust
    #[serde(default)]
    pub inbox: Option<InboxCfg>,
    #[serde(default)]
    pub journal: Option<JournalCfg>,
    #[serde(default)]
    pub schedule: Option<ScheduleCfg>,
```

and the new structs (near `ActionsCfg`):

```rust
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct InboxCfg {
    #[serde(default = "default_inbox_folder")]
    pub folder: String,
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u64,
}
fn default_inbox_folder() -> String { "Inbox".to_string() }
fn default_idle_minutes() -> u64 { 30 }

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct JournalCfg {
    #[serde(default = "default_journal_folder")]
    pub folder: String,
}
fn default_journal_folder() -> String { "journal".to_string() }

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ScheduleCfg {
    #[serde(default = "default_daily_time")]
    pub daily_time: String,
}
fn default_daily_time() -> String { "01:00".to_string() }
```

- [ ] **Step 3: Add validation** — in `Config::validate()`, before the final `Ok(())`:

```rust
        if let Some(inbox) = &self.inbox {
            if inbox.idle_minutes == 0 {
                return Err(ConfigError::Validation(
                    "inbox.idle_minutes must be greater than 0".into(),
                ));
            }
        }
        if let Some(schedule) = &self.schedule {
            if !is_valid_hhmm(&schedule.daily_time) {
                return Err(ConfigError::Validation(format!(
                    "schedule.daily_time '{}' is not valid HH:MM (24h)",
                    schedule.daily_time
                )));
            }
        }
```

and a free function at module scope:

```rust
/// True if `s` is `H:MM` or `HH:MM` 24-hour time. Minute must be two digits.
fn is_valid_hhmm(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 || parts[1].len() != 2 {
        return false;
    }
    let (Ok(h), Ok(m)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) else {
        return false;
    };
    h <= 23 && m <= 59
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p construct-config`
Expected: PASS (new + existing tests).

- [ ] **Step 5: Commit**

```bash
git add crates/construct-config/src/lib.rs
git commit -m "feat(config): [inbox]/[journal]/[schedule] tables + validation"
```

---

### Task 4: Shared loop-guard `is_excluded`

**Files:**
- Create: `crates/construct-engine/src/guard.rs`
- Modify: `crates/construct-engine/src/lib.rs`

A single exclusion check used by ALL triggers so Construct-managed files never trigger processing. Excludes: any file named `_index.md` (or `_index`), anything inside the `journal/` tree, and anything inside the configured managed folder. Paths are vault-relative for the journal/managed checks; the `_index` check is by file stem.

- [ ] **Step 1: Write the failing test** — `crates/construct-engine/src/guard.rs`:

```rust
//! Shared loop-guard: Construct-managed files must never trigger processing.
use std::path::Path;

/// True if `path` (absolute) is a Construct-managed file that must NOT trigger
/// any pipeline. `vault_root` is the vault's absolute path; `journal_folder` and
/// `managed_folder` are vault-relative folder names (e.g. "journal", "Construct").
pub fn is_excluded(
    path: &Path,
    vault_root: &Path,
    journal_folder: &str,
    managed_folder: Option<&str>,
) -> bool {
    // _index notes (any directory) are managed indices.
    if path.file_stem().and_then(|s| s.to_str()) == Some("_index") {
        return true;
    }
    let Ok(rel) = path.strip_prefix(vault_root) else {
        // Outside the vault entirely → exclude (not ours to touch).
        return true;
    };
    let first = rel.components().next().and_then(|c| c.as_os_str().to_str());
    if first == Some(journal_folder) {
        return true;
    }
    if let (Some(mf), Some(f)) = (managed_folder, first) {
        if f == mf {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn vault() -> PathBuf { PathBuf::from("/v") }

    #[test]
    fn excludes_index_notes_anywhere() {
        assert!(is_excluded(&PathBuf::from("/v/Inbox/_index.md"), &vault(), "journal", None));
        assert!(is_excluded(&PathBuf::from("/v/_index.md"), &vault(), "journal", None));
    }

    #[test]
    fn excludes_journal_tree() {
        assert!(is_excluded(&PathBuf::from("/v/journal/2026/06/02.md"), &vault(), "journal", None));
    }

    #[test]
    fn excludes_managed_folder() {
        assert!(is_excluded(&PathBuf::from("/v/Construct/x.md"), &vault(), "journal", Some("Construct")));
    }

    #[test]
    fn allows_normal_notes() {
        assert!(!is_excluded(&PathBuf::from("/v/Inbox/idea.md"), &vault(), "journal", Some("Construct")));
        assert!(!is_excluded(&PathBuf::from("/v/Projects/x.md"), &vault(), "journal", None));
    }

    #[test]
    fn excludes_outside_vault() {
        assert!(is_excluded(&PathBuf::from("/other/x.md"), &vault(), "journal", None));
    }
}
```

- [ ] **Step 2: Wire the module** — add to `crates/construct-engine/src/lib.rs`:

```rust
pub mod guard;
```

(Place alphabetically near the other `pub mod` lines; check the file for existing module declarations and match the style.)

- [ ] **Step 3: Run the tests**

Run: `cargo test -p construct-engine guard`
Expected: PASS (5 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/construct-engine/src/guard.rs crates/construct-engine/src/lib.rs
git commit -m "feat(engine): shared loop-guard is_excluded for all triggers"
```

---

### Task 5: `TriggerEvent` abstraction + `From<VaultEvent>`

**Files:**
- Create: `crates/construct-engine/src/triggers/mod.rs`
- Modify: `crates/construct-engine/src/lib.rs`

`TriggerEvent` is the unified event type the watch loop consumes from any source. Tag/status come from the existing watcher (`VaultEvent`); idle/schedule come from the new sources (Plans 2/3). A `From<VaultEvent>` makes the existing path a one-line map.

- [ ] **Step 1: Write the failing test** — `crates/construct-engine/src/triggers/mod.rs`:

```rust
//! Unified trigger events. Any source (vault watcher, idle poller, scheduler)
//! produces a `TriggerEvent`; the watch loop routes it to an orchestrator.
pub mod idle;
pub mod schedule;

use construct_obsidian::watcher::VaultEvent;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum TriggerEvent {
    /// A note gained a known trigger tag (today's `NoteTagged`).
    Tagged { path: PathBuf, tag: String },
    /// A note's `construct_status` changed to accepted/rejected (today's `StatusChanged`).
    StatusChanged { path: PathBuf, status: String },
    /// An Inbox note has been idle long enough to process (Plan 2).
    IdleNote { path: PathBuf },
    /// A named scheduled job is due to run now (Plan 3), e.g. "daily_summary".
    Scheduled { job: String },
}

impl From<VaultEvent> for TriggerEvent {
    fn from(e: VaultEvent) -> Self {
        match e {
            VaultEvent::NoteTagged { path, tag } => TriggerEvent::Tagged { path, tag },
            VaultEvent::StatusChanged { path, status } => {
                TriggerEvent::StatusChanged { path, status }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_event_maps_to_trigger_event() {
        let ev = VaultEvent::NoteTagged {
            path: PathBuf::from("/v/a.md"),
            tag: "theconstruct/research".into(),
        };
        assert_eq!(
            TriggerEvent::from(ev),
            TriggerEvent::Tagged {
                path: PathBuf::from("/v/a.md"),
                tag: "theconstruct/research".into()
            }
        );
        let ev = VaultEvent::StatusChanged {
            path: PathBuf::from("/v/a.md"),
            status: "accepted".into(),
        };
        assert_eq!(
            TriggerEvent::from(ev),
            TriggerEvent::StatusChanged {
                path: PathBuf::from("/v/a.md"),
                status: "accepted".into()
            }
        );
    }
}
```

> The `pub mod idle;` / `pub mod schedule;` lines reference files created in Tasks 6 and 7. Create empty placeholder files first so this compiles: `echo "" > crates/construct-engine/src/triggers/idle.rs` and same for `schedule.rs` (Tasks 6/7 fill them). Alternatively, comment out those two `pub mod` lines until Tasks 6/7, then uncomment — but the placeholder approach is cleaner.

- [ ] **Step 2: Create placeholder source files** so the module compiles:

```bash
mkdir -p crates/construct-engine/src/triggers
: > crates/construct-engine/src/triggers/idle.rs
: > crates/construct-engine/src/triggers/schedule.rs
```

- [ ] **Step 3: Wire the module** — add to `crates/construct-engine/src/lib.rs`:

```rust
pub mod triggers;
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p construct-engine triggers`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add crates/construct-engine/src/triggers/ crates/construct-engine/src/lib.rs
git commit -m "feat(engine): TriggerEvent abstraction + From<VaultEvent>"
```

---

### Task 6: Idle trigger core — `should_process_inbox_note`

**Files:**
- Modify: `crates/construct-engine/src/triggers/idle.rs`

Pure decision function (no I/O): given a note's text, its mtime, the current time, and the idle threshold, decide whether the idle poller should emit it. Skips notes that already carry a `construct_status` (the no-reprocess-loop guarantee). The directory-walk + mtime reads live in the spawner (Plan 2); this core is unit-tested in isolation.

- [ ] **Step 1: Write the failing test** — replace the contents of `crates/construct-engine/src/triggers/idle.rs`:

```rust
//! Idle-trigger decision core (pure). The poller (Plan 2) supplies file text +
//! mtime; this decides whether a top-level Inbox note is ready to process.
use crate::pipelines::STATUS_KEY;
use chrono::{DateTime, Local};
use construct_obsidian::frontmatter::Note;

/// True if a note should be processed by the Inbox pipeline now:
/// - it has NO `construct_status` field (never been processed → no reprocess loop), AND
/// - its mtime is at least `idle_minutes` older than `now`.
///
/// `mtime` and `now` are local-time instants. Path-based exclusion (`_index`,
/// non-top-level, journal tree) is handled separately by the loop-guard; this
/// function assumes the caller already scoped to top-level Inbox files.
pub fn should_process_inbox_note(
    text: &str,
    mtime: DateTime<Local>,
    now: DateTime<Local>,
    idle_minutes: u64,
) -> bool {
    let note = Note::parse(text);
    if note.get_str(STATUS_KEY).is_some() {
        return false; // already claimed/processed → never re-trigger
    }
    let idle = now.signed_duration_since(mtime);
    idle.num_minutes() >= idle_minutes as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 6, 2, h, m, 0).unwrap()
    }

    #[test]
    fn idle_long_enough_with_no_status_triggers() {
        assert!(should_process_inbox_note("a quick note", t(10, 0), t(10, 30), 30));
    }

    #[test]
    fn not_idle_enough_does_not_trigger() {
        assert!(!should_process_inbox_note("a quick note", t(10, 0), t(10, 29), 30));
    }

    #[test]
    fn note_with_status_never_triggers_even_if_old() {
        let text = "---\nconstruct_status: review\n---\nbody";
        assert!(!should_process_inbox_note(text, t(8, 0), t(23, 0), 30));
    }

    #[test]
    fn exactly_at_threshold_triggers() {
        assert!(should_process_inbox_note("note", t(10, 0), t(10, 30), 30));
    }
}
```

- [ ] **Step 2: Run to confirm fail**

Run: `cargo test -p construct-engine idle`
Expected: FAIL initially only if placeholder lacked the fn — it will now compile and PASS once written. (TDD note: paste the test block first with the `pub fn` body stubbed to `false`, run → see failures, then write the real body. If you paste the whole block at once, run to confirm PASS.)

- [ ] **Step 3: Run the tests**

Run: `cargo test -p construct-engine idle`
Expected: PASS (4 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/construct-engine/src/triggers/idle.rs
git commit -m "feat(engine): idle trigger core should_process_inbox_note"
```

---

### Task 7: Schedule trigger core — `due`

**Files:**
- Modify: `crates/construct-engine/src/triggers/schedule.rs`

Pure catch-up decision: given the last successful run time (or none), the configured daily time, and now, decide whether the scheduled job should fire. The rule: the job should run once per calendar day at-or-after `daily_time`. It is due if the most recent scheduled firing instant `≤ now` is strictly after `last_run` (or there is no `last_run`).

- [ ] **Step 1: Write the failing test** — replace the contents of `crates/construct-engine/src/triggers/schedule.rs`:

```rust
//! Schedule-trigger decision core (pure). Decides whether a daily job is due,
//! including catch-up after downtime. No sleeping; the spawner (Plan 3) drives
//! the poll loop and supplies the clock + last-run timestamp.
use chrono::{DateTime, Local, NaiveTime, TimeZone};

/// The most recent firing instant at-or-before `now` for a job scheduled daily
/// at `daily_time` (local). E.g. if daily_time=01:00 and now=2026-06-02T00:30,
/// the most recent firing is 2026-06-01T01:00.
pub fn last_firing_at_or_before(now: DateTime<Local>, daily_time: NaiveTime) -> DateTime<Local> {
    let today_fire = Local
        .from_local_datetime(&now.date_naive().and_time(daily_time))
        .single()
        .unwrap_or(now);
    if today_fire <= now {
        today_fire
    } else {
        // today's firing is still in the future → the last one was yesterday
        let yest = now.date_naive().pred_opt().unwrap_or(now.date_naive());
        Local
            .from_local_datetime(&yest.and_time(daily_time))
            .single()
            .unwrap_or(now)
    }
}

/// True if the daily job is due to run now. `last_run` is the last successful
/// run instant (parsed from the store), or None if it has never run.
pub fn due(last_run: Option<DateTime<Local>>, daily_time: NaiveTime, now: DateTime<Local>) -> bool {
    let fire = last_firing_at_or_before(now, daily_time);
    match last_run {
        None => true,                 // never run → run now (covers first launch)
        Some(last) => last < fire,    // a firing instant has elapsed since last run
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }
    fn one_am() -> NaiveTime {
        NaiveTime::from_hms_opt(1, 0, 0).unwrap()
    }

    #[test]
    fn never_run_is_due() {
        assert!(due(None, one_am(), at(2026, 6, 2, 9, 0)));
    }

    #[test]
    fn not_due_when_already_ran_after_todays_firing() {
        // ran at 01:05 today; now is 09:00 today; next firing is tomorrow 01:00.
        let last = at(2026, 6, 2, 1, 5);
        assert!(!due(Some(last), one_am(), at(2026, 6, 2, 9, 0)));
    }

    #[test]
    fn catch_up_when_firing_missed() {
        // last ran yesterday 01:00; laptop asleep through today's 01:00;
        // now it is 09:00 today → today's 01:00 firing elapsed, so due.
        let last = at(2026, 6, 1, 1, 0);
        assert!(due(Some(last), one_am(), at(2026, 6, 2, 9, 0)));
    }

    #[test]
    fn not_due_before_first_firing_of_day_if_already_ran() {
        // ran yesterday 01:00; now is today 00:30 (before today's 01:00).
        let last = at(2026, 6, 1, 1, 0);
        assert!(!due(Some(last), one_am(), at(2026, 6, 2, 0, 30)));
    }

    #[test]
    fn last_firing_picks_yesterday_before_todays_time() {
        let now = at(2026, 6, 2, 0, 30);
        assert_eq!(last_firing_at_or_before(now, one_am()), at(2026, 6, 1, 1, 0));
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p construct-engine schedule`
Expected: PASS (5 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/construct-engine/src/triggers/schedule.rs
git commit -m "feat(engine): schedule trigger core (due + catch-up)"
```

---

### Task 8: New `PipelineKind` variants (Inbox, DailySummary) + stub dispatch

**Files:**
- Modify: `crates/construct-engine/src/pipelines/mod.rs`, `crates/construct-engine/src/orchestrator.rs`

Add the two variants now (Plans 2/3 fill in their behavior). `from_name` parses `"inbox"` and `"daily_summary"`. They are NOT auto-apply in the tag sense. Because `handle_tagged` matches exhaustively over `PipelineKind`, add stub arms that fail clearly — they are unreachable until Plans 2/3 wire the sources, but keep the build green.

- [ ] **Step 1: Write the failing test** — in `crates/construct-engine/src/pipelines/mod.rs` `tests` module, extend `pipeline_kind_parses`:

```rust
    #[test]
    fn new_pipeline_kinds_parse() {
        assert_eq!(PipelineKind::from_name("inbox"), Some(PipelineKind::Inbox));
        assert_eq!(
            PipelineKind::from_name("daily_summary"),
            Some(PipelineKind::DailySummary)
        );
        assert!(!PipelineKind::Inbox.is_auto_apply());
        assert!(!PipelineKind::DailySummary.is_auto_apply());
    }
```

- [ ] **Step 2: Run to confirm fail**

Run: `cargo test -p construct-engine new_pipeline_kinds_parse`
Expected: FAIL (no variant `Inbox`).

- [ ] **Step 3: Add the variants** — in `pipelines/mod.rs`:

```rust
pub enum PipelineKind {
    Research,
    Summarize,
    Tag,
    Organize,
    Inbox,
    DailySummary,
}
```

and in `from_name`, add before `_ => None`:

```rust
            "inbox" => Some(PipelineKind::Inbox),
            "daily_summary" => Some(PipelineKind::DailySummary),
```

(`is_auto_apply` already returns false for these via its `matches!` on Summarize|Tag — no change needed.)

- [ ] **Step 4: Add stub dispatch arms** — in `crates/construct-engine/src/orchestrator.rs`, in `handle_tagged`'s `match self.pipeline`, add:

```rust
            PipelineKind::Inbox => {
                self.fail(&run_id, path, "inbox pipeline not yet implemented (Plan 2)")
                    .await
            }
            PipelineKind::DailySummary => {
                self.fail(&run_id, path, "daily summary is scheduled, not tag-triggered")
                    .await
            }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p construct-engine`
Expected: PASS — new test passes AND all existing orchestrator tests still pass (behavior preservation).

- [ ] **Step 6: Commit**

```bash
git add crates/construct-engine/src/pipelines/mod.rs crates/construct-engine/src/orchestrator.rs
git commit -m "feat(engine): add Inbox + DailySummary PipelineKind variants (stub dispatch)"
```

---

### Task 9: Route the watch loop through `TriggerEvent` (behavior-preserving)

**Files:**
- Modify: `crates/construct-cli/src/tui/watch_loop.rs`

Convert the watch loop to consume `TriggerEvent` at its routing core, mapping the watcher's `VaultEvent` via `.into()`. Existing tag/status routing is unchanged. Extract the routing into a small testable helper so behavior preservation is asserted by a unit test.

- [ ] **Step 1: Write the failing test** — add to the `tests` module in `watch_loop.rs`:

```rust
    use construct_engine::triggers::TriggerEvent;
    use std::path::PathBuf;

    #[test]
    fn classify_route_tagged_goes_to_matching_orchestrator_key() {
        // route_key returns Some(tag) for Tagged, None for broadcast/unhandled.
        let ev = TriggerEvent::Tagged { path: PathBuf::from("/v/a.md"), tag: "t/x".into() };
        assert_eq!(route_key(&ev), RouteTarget::Tag("t/x".to_string()));
        let ev = TriggerEvent::StatusChanged { path: PathBuf::from("/v/a.md"), status: "accepted".into() };
        assert_eq!(route_key(&ev), RouteTarget::Broadcast);
        // Idle/Scheduled have no consumer yet in Plan 1 → Unhandled.
        assert_eq!(route_key(&TriggerEvent::IdleNote { path: PathBuf::from("/v/i.md") }), RouteTarget::Unhandled);
        assert_eq!(route_key(&TriggerEvent::Scheduled { job: "daily_summary".into() }), RouteTarget::Unhandled);
    }
```

- [ ] **Step 2: Add the routing helper** — in `watch_loop.rs` (module scope), add:

```rust
use construct_engine::triggers::TriggerEvent;

/// How a trigger event should be routed to orchestrators.
#[derive(Debug, PartialEq)]
enum RouteTarget {
    /// Route to the single orchestrator whose rule matches this tag.
    Tag(String),
    /// Broadcast to all orchestrators (status decisions — any may own the run).
    Broadcast,
    /// No consumer wired yet (idle/schedule land in Plans 2/3).
    Unhandled,
}

fn route_key(ev: &TriggerEvent) -> RouteTarget {
    match ev {
        TriggerEvent::Tagged { tag, .. } => RouteTarget::Tag(tag.clone()),
        TriggerEvent::StatusChanged { .. } => RouteTarget::Broadcast,
        TriggerEvent::IdleNote { .. } | TriggerEvent::Scheduled { .. } => RouteTarget::Unhandled,
    }
}
```

- [ ] **Step 3: Rewire the event loop** — replace the `while let Some(event) = rx.recv().await { match &event { ... } }` body so it maps the incoming `VaultEvent` to a `TriggerEvent` and dispatches via `route_key`. The handler calls remain identical (build per-note lock, spawn `o.handle(VaultEvent...)`). Keep passing the original `VaultEvent` to `o.handle` (the orchestrator API is unchanged in Plan 1). Concretely:

```rust
    while let Some(vault_event) = rx.recv().await {
        let event: TriggerEvent = vault_event.clone().into();
        match route_key(&event) {
            RouteTarget::Tag(tag) => {
                if let Some(o) = orchestrators.get(&tag) {
                    let o = o.clone();
                    let path = match &vault_event {
                        VaultEvent::NoteTagged { path, .. } => path.clone(),
                        VaultEvent::StatusChanged { path, .. } => path.clone(),
                    };
                    let key = path.to_string_lossy().to_string();
                    let lock = {
                        let mut map = note_locks.lock().unwrap();
                        map.entry(key)
                            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                            .clone()
                    };
                    tokio::spawn(async move {
                        let _guard = lock.lock().await;
                        if let Err(e) = o.handle(vault_event).await {
                            tracing::error!("handler error: {e}");
                        }
                    });
                }
            }
            RouteTarget::Broadcast => {
                for o in orchestrators.values() {
                    let o = o.clone();
                    let ev = vault_event.clone();
                    tokio::spawn(async move {
                        let _ = o.handle(ev).await;
                    });
                }
            }
            RouteTarget::Unhandled => {
                tracing::debug!("unhandled trigger event: {event:?}");
            }
        }
    }
```

> The behavior for `NoteTagged` and `StatusChanged` is byte-for-byte equivalent to the original (Tagged → matching orchestrator with per-note lock; StatusChanged → broadcast). Only the dispatch is now expressed through `TriggerEvent`/`route_key`.

- [ ] **Step 4: Run the tests + full build**

Run: `cargo test -p construct-cli && cargo build`
Expected: PASS. Then `cargo clippy --all-targets -- -D warnings` clean (remove the now-unused `match &event` import remnants if any).

- [ ] **Step 5: Commit**

```bash
git add crates/construct-cli/src/tui/watch_loop.rs
git commit -m "refactor(cli): route watch loop through TriggerEvent (behavior-preserving)"
```

---

### Task 10: Feature C — stats access documentation (docs only)

**Files:**
- Create: `docs/stats-access.md`
- Modify: `README.md` (add a short pointer link to the new doc)

No runtime code. Document the `construct.db` schema and example read-only queries for a future monitoring app / `ssh + sqlite3`.

- [ ] **Step 1: Write the doc** — `docs/stats-access.md`:

````markdown
# The Construct — Activity & Stats Access

The Construct records every pipeline run and its lifecycle events in a local SQLite
database, `construct.db`, in the working directory where `entertheconstruct` runs.
A future monitoring app (or a quick `ssh + sqlite3` session) can read it directly.
**This data is read-only for outside consumers — never write to `construct.db`
from another process while The Construct is running.**

## Schema

### `runs` — one row per pipeline run
| column      | type | meaning |
|-------------|------|---------|
| `id`        | TEXT PK | run UUID (also stamped into the note as `construct_run_id`) |
| `rule`      | TEXT | pipeline/rule that owns the run (e.g. `research`, `tag`, `inbox`) |
| `agent`     | TEXT | agent name (e.g. `Scout`, `Librarian`) |
| `note_path` | TEXT | absolute path of the note (or generated day note) |
| `status`    | TEXT | `queued`/`running`/`researching`/`review`/`accepted`/`rejected`/`done`/`error` |
| `error`     | TEXT | error message when `status = error`, else NULL |
| `created_at`| TEXT | UTC datetime the run was created |
| `updated_at`| TEXT | UTC datetime of the last status change |

### `run_events` — append-only timeline per run
| column     | type | meaning |
|------------|------|---------|
| `id`       | INTEGER PK AUTOINCREMENT | event sequence id |
| `run_id`   | TEXT FK→runs.id | the run this event belongs to |
| `stage`    | TEXT | pipeline stage (`claim`, `summarize`, `write_back`, `error`, …) |
| `event`    | TEXT | short event label (`queued`, `done`, `review`, `failed`, …) |
| `payload`  | TEXT | JSON blob with stage-specific detail |
| `ts`       | TEXT | UTC datetime of the event |

### `schedule_state` — last-run bookkeeping for scheduled jobs
| column     | type | meaning |
|------------|------|---------|
| `job`      | TEXT PK | scheduled job name (e.g. `daily_summary`) |
| `last_run` | TEXT | RFC3339 timestamp of the last successful run |

## Example read-only queries

Recent activity feed (latest events with their run context):
```sql
SELECT e.ts, r.rule, r.agent, e.stage, e.event, r.note_path
FROM run_events e JOIN runs r ON r.id = e.run_id
ORDER BY e.id DESC
LIMIT 50;
```

Runs grouped by status:
```sql
SELECT status, COUNT(*) AS n FROM runs GROUP BY status ORDER BY n DESC;
```

Runs grouped by pipeline and day:
```sql
SELECT date(created_at) AS day, rule, COUNT(*) AS n
FROM runs GROUP BY day, rule ORDER BY day DESC, n DESC;
```

Notes currently awaiting human review:
```sql
SELECT note_path, rule, agent, updated_at
FROM runs WHERE status = 'review' ORDER BY updated_at DESC;
```

Quick CLI peek over SSH:
```sh
ssh my-host "sqlite3 -readonly ~/path/to/construct.db \
  'SELECT status, COUNT(*) FROM runs GROUP BY status;'"
```

## Out of scope (backlog)
A read-only `entertheconstruct stats` subcommand and any HTTP/monitoring server are
explicitly deferred. This document is the contract a separate app would build against.
````

- [ ] **Step 2: Add a README pointer** — in `README.md`, under an appropriate section (e.g. near where the database or architecture is mentioned), add:

```markdown
For reading The Construct's activity/run data (schema + example queries), see
[docs/stats-access.md](docs/stats-access.md).
```

- [ ] **Step 3: Commit**

```bash
git add docs/stats-access.md README.md
git commit -m "docs: Feature C stats access — construct.db schema + example queries"
```

---

## Final verification (run after all tasks)

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

All three must be green. Then confirm behavior preservation: the existing orchestrator
and watcher tests pass unchanged, and no rule can select `inbox`/`daily_summary`
(they are reached only via the idle/schedule sources wired in Plans 2 & 3).

## Self-Review checklist (done at plan-write time)
- **Spec coverage:** Trigger abstraction ✅ (Task 5); IdleTrigger core ✅ (6); ScheduleTrigger core + catch-up ✅ (7) + store state (2); clock injection ✅ (1); config tables + validation ✅ (3); loop guard ✅ (4); new PipelineKinds ✅ (8); behavior-preserving watch loop ✅ (9); Feature C docs ✅ (10). The idle/schedule *spawners that wire into the running loop* are intentionally deferred to Plans 2/3 (a source with no pipeline consumer is not shippable software on its own).
- **Type consistency:** `Clock::now_local`, `should_process_inbox_note`, `due`/`last_firing_at_or_before`, `is_excluded`, `TriggerEvent`, `RouteTarget`/`route_key`, `get_last_run`/`set_last_run`, `InboxCfg`/`JournalCfg`/`ScheduleCfg` names are used consistently across tasks and into Plans 2/3.
- **No placeholders:** every code step contains real code.
