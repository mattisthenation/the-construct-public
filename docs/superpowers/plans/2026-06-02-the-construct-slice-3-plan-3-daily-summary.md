# Slice 3 — Plan 3: Daily Summary (Scheduled)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** At `schedule.daily_time` (default 01:00 local, with catch-up if missed), generate/update a journal day note `journal/YYYY/MM/DD.md` with four idempotent managed sections: **Today's Task List** (open `- [ ]` checkboxes scraped deterministically from yesterday's changed notes + carryover, de-duped), **Carryover from yesterday** (still-unchecked items from yesterday's journal day-note), **Yesterday summary** (LLM prose recap, gated), and **Other notes** (links to yesterday's changed notes not otherwise captured).

**Architecture:** Builds on Plan 1's `Clock`, `schedule::due`/`last_firing_at_or_before`, `Store::get/set_last_run`, loop-guard, and `PipelineKind::DailySummary`. Adds pure date/checkbox/section helpers in a new `pipelines/daily.rs`, a recursive note walker in `vault.rs`, an `HH:MM` parser, the `run_daily_summary` orchestrator method (takes an injected `today: NaiveDate` so it is fully testable without a real clock), and the watch-loop scheduler (build a DailySummary orchestrator + a 60s poll task that fires `run_daily_summary` when due and records `last_run` for catch-up).

**Key behavioral decisions (read before coding):**
- **Determinism split:** task list + carryover are scraped deterministically (local models invent tasks); only the prose recap is agentic.
- **Loop-guard:** the journal tree is excluded by `is_excluded`, so generated day notes never trigger Inbox/tag/organize.
- **Idempotency:** re-running for the same day updates the day note's four managed blocks in place (via `upsert_named`), never duplicating. `run_daily_summary` writes ONLY managed blocks to the day note — it does NOT stamp `construct_status` frontmatter on journal notes.
- **`run_daily_summary` is standalone** (not routed through `handle_tagged`/claim). It creates its own store run (note_path = the day note) for observability, runs Running→Done. Because journal notes must never carry a claim, `reconcile()` skips DailySummary runs (a missed run recovers on the next schedule fire / catch-up, not via note re-claim).
- **First launch:** `due(None, …)` returns true (Plan 1, deliberate) → the first run after install fires immediately and summarizes "yesterday". Acceptable.
- **Missing yesterday journal note → empty Carryover section, not an error.** No changed notes → prose is a fixed "No notes changed yesterday." string (no LLM call).
- **All times local.**

**Tech Stack:** Rust workspace, chrono (`NaiveDate`/`NaiveTime`/`Local`), tokio, sqlx, filetime (dev). Reuses `run_loop`, `validate_summary` (for the prose), managed blocks (`upsert_named`), `is_excluded`, `Store`. Commit author `Matt <matt@matthewlittlehale.com>` with trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

**Conventions:** TDD per task. Keep `cargo test`/`clippy --all-targets -- -D warnings`/`fmt --all -- --check` green at every commit.

---

## File Structure

- `crates/construct-obsidian/src/vault.rs` — **MODIFY**: add `walk_notes`.
- `crates/construct-engine/src/pipelines/daily.rs` — **NEW**: `journal_day_path`, `scrape_open_checkboxes`, `dedupe_preserving_order`, `render_day_note`, `changed_notes_on`.
- `crates/construct-engine/src/pipelines/mod.rs` — **MODIFY**: `pub mod daily;`.
- `crates/construct-engine/src/triggers/schedule.rs` — **MODIFY**: add `parse_hhmm`.
- `crates/construct-engine/src/orchestrator.rs` — **MODIFY**: `run_daily_summary` + reconcile skip for DailySummary.
- `crates/construct-cli/src/tui/watch_loop.rs` — **MODIFY**: build DailySummary orchestrator + spawn scheduler poll task.

---

### Task 1: `walk_notes` recursive note lister

**Files:**
- Modify: `crates/construct-obsidian/src/vault.rs`

Recursively list all `.md` files under `root` (absolute paths), skipping dotfolders and any directory named in `exclude`. Mirrors the existing `collect_tags` walk shape.

- [ ] **Step 1: Write the failing test** — add to the `tests` module in `vault.rs`:

```rust
    #[test]
    fn walk_notes_lists_md_recursively_skipping_dot_and_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("a.md"), "x");
        write(&root.join("sub/b.md"), "y");
        write(&root.join(".obsidian/c.md"), "z");
        write(&root.join("Archive/old.md"), "w");
        write(&root.join("notes.txt"), "not md");
        let mut found: Vec<String> = walk_notes(root, &["Archive".to_string()])
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().to_string())
            .collect();
        found.sort();
        assert_eq!(found, vec!["a.md".to_string(), "sub/b.md".to_string()]);
    }
```

- [ ] **Step 2: Run to confirm fail**

Run: `cargo test -p construct-obsidian walk_notes`
Expected: FAIL (function not found).

- [ ] **Step 3: Implement** — add to `vault.rs`:

```rust
/// Recursively list all `.md` files under `root` (absolute paths), skipping
/// dotfolders and any directory whose name appears in `exclude`.
pub fn walk_notes(root: &Path, exclude: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_notes(root, exclude, &mut out);
    out
}

fn collect_notes(dir: &Path, exclude: &[String], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !name.starts_with('.') && !exclude.iter().any(|e| e == name) {
                collect_notes(&path, exclude, out);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}
```

(Remove the now-unneeded `#[allow(dead_code)] fn _types` stub if it is still present and clippy complains; otherwise leave it.)

- [ ] **Step 4: Run the test**

Run: `cargo test -p construct-obsidian walk_notes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git -c user.name=Matt -c user.email=matt@matthewlittlehale.com commit -am "feat(obsidian): walk_notes recursive .md lister" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```
(Stage only `vault.rs`: `git add crates/construct-obsidian/src/vault.rs` then commit.)

---

### Task 2: `daily.rs` — `journal_day_path` + `scrape_open_checkboxes`

**Files:**
- Create: `crates/construct-engine/src/pipelines/daily.rs`
- Modify: `crates/construct-engine/src/pipelines/mod.rs`

- [ ] **Step 1: Write the failing test** — `crates/construct-engine/src/pipelines/daily.rs`:

```rust
//! Daily-summary pipeline helpers: journal path math, deterministic checkbox
//! scraping, section assembly, and the "changed yesterday" note scan.
use chrono::{Datelike, NaiveDate};
use std::path::PathBuf;

/// Vault-relative path of the journal day note for `date`:
/// `<journal_folder>/YYYY/MM/DD.md` with zero-padded month and day.
pub fn journal_day_path(journal_folder: &str, date: NaiveDate) -> PathBuf {
    PathBuf::from(journal_folder)
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{:02}.md", date.day()))
}

/// Extract the text of every OPEN checkbox (`- [ ]`) line, trimmed. Checked
/// (`- [x]`) items are ignored. Order preserved.
pub fn scrape_open_checkboxes(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("- [ ]") {
            let task = rest.trim();
            if !task.is_empty() {
                out.push(task.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn journal_path_is_zero_padded() {
        assert_eq!(
            journal_day_path("journal", d(2026, 6, 2)),
            PathBuf::from("journal/2026/06/02.md")
        );
    }

    #[test]
    fn journal_path_year_rollover() {
        assert_eq!(
            journal_day_path("journal", d(2027, 1, 1)),
            PathBuf::from("journal/2027/01/01.md")
        );
        assert_eq!(
            journal_day_path("journal", d(2026, 12, 31)),
            PathBuf::from("journal/2026/12/31.md")
        );
    }

    #[test]
    fn scrape_open_checkboxes_ignores_checked_and_empty() {
        let text = "intro\n- [ ] buy milk\n  - [ ] nested task\n- [x] done thing\n- [ ]   \n- not a checkbox";
        assert_eq!(
            scrape_open_checkboxes(text),
            vec!["buy milk".to_string(), "nested task".to_string()]
        );
    }
}
```

- [ ] **Step 2: Register the module** — in `pipelines/mod.rs`, add:

```rust
pub mod daily;
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p construct-engine daily`
Expected: PASS (3 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/construct-engine/src/pipelines/daily.rs crates/construct-engine/src/pipelines/mod.rs
git -c user.name=Matt -c user.email=matt@matthewlittlehale.com commit -m "feat(engine): daily journal path + open-checkbox scraper" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: `daily.rs` — `dedupe_preserving_order` + `render_day_note`

**Files:**
- Modify: `crates/construct-engine/src/pipelines/daily.rs`

`render_day_note` assembles the four managed sections into the day note body idempotently. Each section is a named managed block, so re-running updates in place. Input is the CURRENT day-note text (`""` if new) and the four computed pieces; output is the new full text.

- [ ] **Step 1: Write the failing tests** — add to the `tests` module in `daily.rs`:

```rust
    #[test]
    fn dedupe_preserves_first_occurrence_order() {
        let v = vec!["a".to_string(), "b".to_string(), "a".to_string(), "c".to_string()];
        assert_eq!(dedupe_preserving_order(v), vec!["a", "b", "c"]);
    }

    #[test]
    fn render_day_note_writes_four_sections_and_is_idempotent() {
        let tasks = vec!["buy milk".to_string(), "call Sam".to_string()];
        let carry = vec!["old task".to_string()];
        let prose = "You edited two notes about the project.";
        let others = vec!["[[Project Plan]]".to_string()];

        let once = render_day_note("", &tasks, &carry, prose, &others);
        // All four blocks present.
        assert!(once.contains("construct:daily-tasks:start"));
        assert!(once.contains("construct:daily-carryover:start"));
        assert!(once.contains("construct:daily-summary:start"));
        assert!(once.contains("construct:daily-other:start"));
        // Section content.
        assert!(once.contains("- [ ] buy milk"));
        assert!(once.contains("- [ ] old task"));
        assert!(once.contains("You edited two notes"));
        assert!(once.contains("[[Project Plan]]"));
        // Headings present (human-readable).
        assert!(once.contains("Today's Task List"));
        assert!(once.contains("Carryover"));

        // Re-render updates in place: still exactly one of each block.
        let twice = render_day_note(&once, &["buy milk".to_string()], &[], "New prose.", &[]);
        assert_eq!(twice.matches("construct:daily-tasks:start").count(), 1);
        assert_eq!(twice.matches("construct:daily-summary:start").count(), 1);
        assert!(twice.contains("New prose."));
        assert!(!twice.contains("You edited two notes"));
    }
```

- [ ] **Step 2: Implement** — add to `daily.rs` (above `#[cfg(test)]`):

```rust
use construct_obsidian::block::upsert_named;

/// De-duplicate while preserving first-occurrence order.
pub fn dedupe_preserving_order(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items.into_iter().filter(|i| seen.insert(i.clone())).collect()
}

/// Build/update the four managed sections of a journal day note. `current` is the
/// existing day-note text ("" for a new note). Returns the full new text. Each
/// section is a named managed block so re-running updates in place.
pub fn render_day_note(
    current: &str,
    tasks: &[String],
    carryover: &[String],
    prose: &str,
    other_links: &[String],
) -> String {
    let tasks_body = render_checkbox_section("Today's Task List", tasks);
    let carry_body = render_checkbox_section("Carryover from yesterday", carryover);
    let summary_body = format!("## Yesterday summary\n\n{}", prose.trim());
    let other_body = render_links_section("Other notes", other_links);

    let mut body = upsert_named(current, "daily-tasks", &tasks_body);
    body = upsert_named(&body, "daily-carryover", &carry_body);
    body = upsert_named(&body, "daily-summary", &summary_body);
    body = upsert_named(&body, "daily-other", &other_body);
    body
}

fn render_checkbox_section(heading: &str, items: &[String]) -> String {
    let mut s = format!("## {heading}\n");
    if items.is_empty() {
        s.push_str("\n_None._\n");
    } else {
        for it in items {
            s.push_str(&format!("- [ ] {it}\n"));
        }
    }
    s
}

fn render_links_section(heading: &str, links: &[String]) -> String {
    let mut s = format!("## {heading}\n");
    if links.is_empty() {
        s.push_str("\n_None._\n");
    } else {
        for l in links {
            s.push_str(&format!("- {l}\n"));
        }
    }
    s
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p construct-engine daily`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/construct-engine/src/pipelines/daily.rs
git -c user.name=Matt -c user.email=matt@matthewlittlehale.com commit -m "feat(engine): daily day-note section assembly (idempotent managed blocks)" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: `daily.rs` — `changed_notes_on`

**Files:**
- Modify: `crates/construct-engine/src/pipelines/daily.rs`

Scan the vault for `.md` notes whose filesystem **mtime falls on `date`** (local calendar day), excluding the journal tree, managed folder, and `_index`/managed files via the shared loop-guard.

- [ ] **Step 1: Write the failing test** — add to the `tests` module in `daily.rs`:

```rust
    use chrono::Local;

    fn set_mtime(path: &std::path::Path, dt: chrono::DateTime<Local>) {
        let ft = filetime::FileTime::from_unix_time(dt.timestamp(), 0);
        filetime::set_file_mtime(path, ft).unwrap();
    }

    #[test]
    fn changed_notes_on_filters_by_day_and_loop_guard() {
        use chrono::TimeZone;
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        std::fs::create_dir_all(vault.join("journal/2026/06")).unwrap();
        std::fs::create_dir_all(vault.join("Projects")).unwrap();
        let yesterday = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();

        // changed yesterday → included
        let a = vault.join("Projects/a.md");
        std::fs::write(&a, "edited yesterday").unwrap();
        set_mtime(&a, Local.with_ymd_and_hms(2026, 6, 1, 14, 0, 0).unwrap());

        // changed today → excluded
        let b = vault.join("Projects/b.md");
        std::fs::write(&b, "edited today").unwrap();
        set_mtime(&b, Local.with_ymd_and_hms(2026, 6, 2, 9, 0, 0).unwrap());

        // a journal note changed yesterday → excluded (loop guard)
        let j = vault.join("journal/2026/06/01.md");
        std::fs::write(&j, "journal").unwrap();
        set_mtime(&j, Local.with_ymd_and_hms(2026, 6, 1, 1, 0, 0).unwrap());

        // an _index changed yesterday → excluded
        let idx = vault.join("Projects/_index.md");
        std::fs::write(&idx, "index").unwrap();
        set_mtime(&idx, Local.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap());

        let found = changed_notes_on(vault, &[], "journal", None, yesterday);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.md".to_string()]);
    }
```

- [ ] **Step 2: Implement** — add to `daily.rs`:

```rust
use crate::guard::is_excluded;
use chrono::{DateTime, Local};
use std::path::Path;

/// All vault notes whose filesystem mtime falls on the local calendar day `date`,
/// excluding journal/managed/_index files (shared loop-guard) and `exclude_dirs`.
pub fn changed_notes_on(
    vault_root: &Path,
    exclude_dirs: &[String],
    journal_folder: &str,
    managed_folder: Option<&str>,
    date: NaiveDate,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in construct_obsidian::vault::walk_notes(vault_root, exclude_dirs) {
        if is_excluded(&path, vault_root, journal_folder, managed_folder) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        let Ok(modified) = meta.modified() else { continue };
        let mtime: DateTime<Local> = modified.into();
        if mtime.date_naive() == date {
            out.push(path);
        }
    }
    out.sort();
    out
}
```

> Note: `is_excluded` already excludes the `journal/` tree, but `walk_notes` will still descend into it; the guard filters it out. To avoid even reading journal files, callers MAY pass the journal folder in `exclude_dirs` — but the guard is the correctness backstop, so this is optional.

- [ ] **Step 3: Run the test**

Run: `cargo test -p construct-engine changed_notes_on`
Expected: PASS. (`filetime` dev-dependency was added in Plan 2 Task 7; if this is built independently and filetime is missing, run `cargo add filetime --dev -p construct-engine`.)

- [ ] **Step 4: Commit**

```bash
git add crates/construct-engine/src/pipelines/daily.rs
git -c user.name=Matt -c user.email=matt@matthewlittlehale.com commit -m "feat(engine): changed_notes_on (yesterday's edited notes, loop-guarded)" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: `parse_hhmm` in schedule.rs

**Files:**
- Modify: `crates/construct-engine/src/triggers/schedule.rs`

Parse a config `daily_time` string (`H:MM`/`HH:MM`) into a `NaiveTime`. Config already validated the format (Plan 1), but the watch loop needs the parsed value.

- [ ] **Step 1: Write the failing test** — add to the `tests` module in `schedule.rs`:

```rust
    #[test]
    fn parse_hhmm_parses_valid_times() {
        assert_eq!(parse_hhmm("01:00"), NaiveTime::from_hms_opt(1, 0, 0));
        assert_eq!(parse_hhmm("9:05"), NaiveTime::from_hms_opt(9, 5, 0));
        assert_eq!(parse_hhmm("23:59"), NaiveTime::from_hms_opt(23, 59, 0));
        assert_eq!(parse_hhmm("bad"), None);
        assert_eq!(parse_hhmm("25:00"), None);
    }
```

- [ ] **Step 2: Implement** — add to `schedule.rs`:

```rust
/// Parse a `H:MM`/`HH:MM` 24-hour string into a `NaiveTime` (seconds = 0).
pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    NaiveTime::from_hms_opt(h, m, 0)
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p construct-engine parse_hhmm`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/construct-engine/src/triggers/schedule.rs
git -c user.name=Matt -c user.email=matt@matthewlittlehale.com commit -m "feat(engine): parse_hhmm for schedule daily_time" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: Orchestrator — `run_daily_summary` + reconcile skip

**Files:**
- Modify: `crates/construct-engine/src/orchestrator.rs`

`run_daily_summary(today)` is standalone (no claim). It scans yesterday's changed notes, builds the four sections, writes the day note (managed blocks only), and records a Running→Done store run. `reconcile` skips DailySummary (journal notes must never be re-claimed).

- [ ] **Step 1: Write the failing integration test** — add to the `tests` module in `orchestrator.rs` (reuse `ScriptedModel`/`chat_text`/`EchoTool`/in-memory store; copy the `Orchestrator{...}` field list from an existing test, set `pipeline: PipelineKind::DailySummary`, `rule: "daily_summary".into()`):

```rust
    #[tokio::test]
    async fn daily_summary_builds_day_note_with_sections_and_carryover() {
        use crate::pipelines::daily;
        use crate::pipelines::PipelineKind;
        use chrono::{Local, NaiveDate, TimeZone};

        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let today = NaiveDate::from_ymd_opt(2026, 6, 2).unwrap();
        let yesterday = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();

        // A note edited yesterday with an open checkbox.
        std::fs::create_dir_all(vault.join("Projects")).unwrap();
        let n = vault.join("Projects/plan.md");
        std::fs::write(&n, "# Plan\n- [ ] ship slice 3\n- [x] write spec\n").unwrap();
        let ft = filetime::FileTime::from_unix_time(
            Local.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap().timestamp(),
            0,
        );
        filetime::set_file_mtime(&n, ft).unwrap();

        // Yesterday's journal day-note with an unchecked carryover item.
        let yest_journal = vault.join(daily::journal_day_path("journal", yesterday));
        std::fs::create_dir_all(yest_journal.parent().unwrap()).unwrap();
        std::fs::write(&yest_journal, "## Today's Task List\n- [ ] leftover task\n").unwrap();

        // Prose recap (the only agentic step).
        let model = ScriptedModel::new(vec![chat_text(
            r#"{"tldr":"You worked on the slice 3 plan.","action_items":[]}"#,
        )]);
        let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let store: Arc<dyn Store> =
            Arc::new(SqliteStore::connect("sqlite::memory:").await.unwrap());
        let orch = Orchestrator {
            store: store.clone(),
            provider: Arc::new(model),
            tools,
            model: "m".into(),
            agent: "Librarian".into(),
            rule: "daily_summary".into(),
            pipeline: PipelineKind::DailySummary,
            system_prompt: "You are the Librarian.".into(),
            max_iterations: 4,
            done_tag: None,
            vault_path: vault.to_path_buf(),
            max_tags: 8,
            exclude_dirs: vec![],
        };

        orch.run_daily_summary(today, "journal").await.unwrap();

        let day_note = vault.join(daily::journal_day_path("journal", today));
        assert!(day_note.exists());
        let text = std::fs::read_to_string(&day_note).unwrap();
        // Task scraped from yesterday's changed note.
        assert!(text.contains("- [ ] ship slice 3"));
        // Carryover from yesterday's journal note.
        assert!(text.contains("- [ ] leftover task"));
        // Prose recap.
        assert!(text.contains("You worked on the slice 3 plan."));
        // Link to the changed note under "Other notes".
        assert!(text.contains("[[plan]]"));
        // Checked item not scraped.
        assert!(!text.contains("write spec"));

        // Idempotent re-run: still one of each block.
        orch.run_daily_summary(today, "journal").await.unwrap();
        let text2 = std::fs::read_to_string(&day_note).unwrap();
        assert_eq!(text2.matches("construct:daily-tasks:start").count(), 1);
    }
```

- [ ] **Step 2: Run to confirm fail**

Run: `cargo test -p construct-engine daily_summary_builds_day_note`
Expected: FAIL (`run_daily_summary` not found).

- [ ] **Step 3: Implement `run_daily_summary`** — add to `impl Orchestrator`:

```rust
    /// Daily summary pipeline (scheduled). Standalone: creates its own store run,
    /// scans yesterday's changed notes, and writes/updates today's journal day note
    /// with four managed sections. `today` is injected (from the scheduler's clock)
    /// so this is fully testable. Writes ONLY managed blocks — no frontmatter claim.
    pub async fn run_daily_summary(
        &self,
        today: chrono::NaiveDate,
        journal_folder: &str,
    ) -> anyhow::Result<()> {
        use crate::pipelines::daily;
        let yesterday = today.pred_opt().unwrap_or(today);
        let managed = None; // managed-folder exclusion handled by exclude_dirs if set

        let day_rel = daily::journal_day_path(journal_folder, today);
        let day_note = self.vault_path.join(&day_rel);
        let note_path_str = day_note.to_string_lossy().to_string();

        // Store run for observability (no note claim).
        let run_id = RunId::new();
        self.store
            .create_run(&RunRecord {
                id: run_id.clone(),
                rule: self.rule.clone(),
                agent: self.agent.clone(),
                note_path: note_path_str.clone(),
                status: RunStatus::Running,
                error: None,
            })
            .await?;
        self.store
            .append_event(&run_id, "daily", "running", serde_json::json!({"day": today.to_string()}))
            .await?;

        // 1. Scan yesterday's changed notes (loop-guarded).
        let changed = daily::changed_notes_on(
            &self.vault_path,
            &self.exclude_dirs,
            journal_folder,
            managed,
            yesterday,
        );

        // 2. Deterministic tasks: open checkboxes from changed notes + carryover.
        let mut note_tasks: Vec<String> = Vec::new();
        let mut contributing: std::collections::HashSet<String> = std::collections::HashSet::new();
        for p in &changed {
            if let Ok(text) = std::fs::read_to_string(p) {
                let tasks = daily::scrape_open_checkboxes(&text);
                if !tasks.is_empty() {
                    contributing.insert(p.to_string_lossy().to_string());
                }
                note_tasks.extend(tasks);
            }
        }
        // Carryover: still-unchecked items from yesterday's journal day-note.
        let yest_journal = self.vault_path.join(daily::journal_day_path(journal_folder, yesterday));
        let carryover = match std::fs::read_to_string(&yest_journal) {
            Ok(t) => daily::scrape_open_checkboxes(&t),
            Err(_) => Vec::new(), // missing yesterday note → empty carryover, not an error
        };
        let mut all_tasks = note_tasks.clone();
        all_tasks.extend(carryover.clone());
        let today_tasks = daily::dedupe_preserving_order(all_tasks);

        // 3. Prose recap (agentic) — skip the model entirely if nothing changed.
        let prose = if changed.is_empty() {
            "No notes changed yesterday.".to_string()
        } else {
            let titles: Vec<String> = changed
                .iter()
                .map(|p| p.file_stem().unwrap_or_default().to_string_lossy().to_string())
                .collect();
            let messages = vec![
                ChatMessage::system(&self.system_prompt),
                ChatMessage::user(format!(
                    "Write a short prose recap (2-4 sentences) of what these notes that changed \
                     yesterday are about: {}. Return STRICT JSON only: \
                     {{\"tldr\": string, \"action_items\": [string]}}.",
                    titles.join(", ")
                )),
            ];
            match run_loop(
                self.provider.as_ref(),
                &self.tools,
                messages,
                &LoopConfig { model: self.model.clone(), max_iterations: self.max_iterations },
            )
            .await
            {
                Ok(r) => match crate::gate::validate_summary(&r.content) {
                    Ok(s) => s.tldr,
                    Err(e) => return self.fail(&run_id, &day_note, &e.to_string()).await,
                },
                Err(e) => return self.fail(&run_id, &day_note, &e.to_string()).await,
            }
        };

        // 4. Other notes: links to changed notes that contributed no task.
        let other_links: Vec<String> = changed
            .iter()
            .filter(|p| !contributing.contains(&p.to_string_lossy().to_string()))
            .map(|p| format!("[[{}]]", p.file_stem().unwrap_or_default().to_string_lossy()))
            .collect();

        // 5. Ensure journal dirs exist, render + write the day note (managed blocks only).
        if let Some(parent) = day_note.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let current = std::fs::read_to_string(&day_note).unwrap_or_default();
        let updated = daily::render_day_note(&current, &today_tasks, &carryover, &prose, &other_links);
        std::fs::write(&day_note, updated)?;

        self.store.update_status(&run_id, RunStatus::Done, None).await?;
        self.store
            .append_event(
                &run_id,
                "daily",
                "done",
                serde_json::json!({"tasks": today_tasks.len(), "changed": changed.len()}),
            )
            .await?;
        Ok(())
    }
```

> **Journal folder:** passed as a parameter (`journal_folder: &str`) rather than hardcoded or added to the `Orchestrator` struct. This honors a custom `[journal] folder` without rippling a new struct field across every `Orchestrator{...}` construction site. The watch loop (Task 7) passes `cfg.journal.folder` (default `"journal"`); the test passes `"journal"`.

- [ ] **Step 4: Add the reconcile skip** — in `reconcile()`, at the very top of the method body (before the `for status in [...]` loop):

```rust
        // Daily-summary runs are scheduled, not note-claim based; they recover on the
        // next schedule fire (with catch-up), never via note re-claim. Skip them here
        // so reconcile never stamps a claim onto a journal day note.
        if self.pipeline == crate::pipelines::PipelineKind::DailySummary {
            return Ok(());
        }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p construct-engine`
Expected: PASS — the new daily-summary integration test AND all existing tests (the reconcile change only adds an early return for a pipeline no existing test uses).

- [ ] **Step 6: Commit**

```bash
git add crates/construct-engine/src/orchestrator.rs
git -c user.name=Matt -c user.email=matt@matthewlittlehale.com commit -m "feat(engine): run_daily_summary pipeline + reconcile skip for scheduled runs" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7: Watch-loop — build DailySummary orchestrator + scheduler poll task

**Files:**
- Modify: `crates/construct-cli/src/tui/watch_loop.rs`

When `cfg.schedule` is present, build a DailySummary `Orchestrator` (rule `"daily_summary"`, pipeline `DailySummary`, agent resolved like the inbox) and spawn a 60s poll task: on each tick read `last_run` from the store, and if `schedule::due(last_run, daily_time, now)` → call `run_daily_summary(now.date_naive())`, then on success `store.set_last_run("daily_summary", now.to_rfc3339())`. Catch-up is automatic (the first tick after launch evaluates `due` against the persisted `last_run`).

- [ ] **Step 1: Build the DailySummary orchestrator** — after the inbox orchestrator block (Plan 2 Task 8), add an analogous block:

```rust
    // Build the DailySummary orchestrator (Feature B) when [schedule] is configured.
    let daily_orch: Option<Arc<Orchestrator>> = if cfg.schedule.is_some() {
        let agent = cfg
            .rules
            .iter()
            .find(|r| r.pipeline == "summarize" || r.pipeline == "tag")
            .and_then(|r| cfg.agent(&r.agent))
            .or_else(|| cfg.agents.first());
        match agent {
            None => {
                tracing::warn!("[schedule] configured but no agent available; skipping daily summary");
                None
            }
            Some(agent) => {
                let provider: Arc<dyn ModelProvider> =
                    Arc::new(OllamaProvider::new(agent.base_url.clone()));
                let system_prompt = agent
                    .system_prompt_file
                    .as_ref()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .unwrap_or_else(|| {
                        format!("You are {}, writing a concise daily journal recap. Always answer with strict JSON.", agent.name)
                    });
                Some(Arc::new(Orchestrator {
                    store: store.clone(),
                    provider,
                    tools: HashMap::new(),
                    model: agent.model.clone(),
                    agent: agent.name.clone(),
                    rule: "daily_summary".into(),
                    pipeline: PipelineKind::DailySummary,
                    system_prompt,
                    max_iterations: 8,
                    done_tag: None,
                    vault_path: shellexpand_tilde(&cfg.vault.path).into(),
                    max_tags: cfg.actions.tag.max_tags,
                    exclude_dirs: cfg.actions.organize.exclude_dirs.clone(),
                }))
            }
        }
    } else {
        None
    };
```

- [ ] **Step 2: Spawn the scheduler poll task** — after the idle poller spawn (Plan 2 Task 8 Step 5), add:

```rust
    // Daily-summary scheduler: poll every 60s; fire run_daily_summary when due
    // (with catch-up via the persisted last_run). All times local.
    if let (Some(sched_cfg), Some(daily)) = (cfg.schedule.clone(), daily_orch.clone()) {
        let store_sched = store.clone();
        let cfg_journal_folder = cfg
            .journal
            .as_ref()
            .map(|j| j.folder.clone())
            .unwrap_or_else(|| "journal".to_string());
        tokio::spawn(async move {
            use construct_core::clock::{Clock, SystemClock};
            use construct_engine::triggers::schedule;
            let clock = SystemClock;
            let Some(daily_time) = schedule::parse_hhmm(&sched_cfg.daily_time) else {
                tracing::warn!("invalid schedule.daily_time '{}'; daily summary disabled", sched_cfg.daily_time);
                return;
            };
            let journal_folder = cfg_journal_folder; // captured String (see below)
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                ticker.tick().await;
                let now = clock.now_local();
                let last_run = store_sched
                    .get_last_run("daily_summary")
                    .await
                    .ok()
                    .flatten()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Local));
                if schedule::due(last_run, daily_time, now) {
                    match daily.run_daily_summary(now.date_naive(), &journal_folder).await {
                        Ok(()) => {
                            let _ = store_sched
                                .set_last_run("daily_summary", &now.to_rfc3339())
                                .await;
                        }
                        Err(e) => tracing::error!("daily summary failed: {e}"),
                    }
                }
            }
        });
    }
```

> The DailySummary orchestrator is driven directly by this task; it is NOT routed through the `TriggerEvent` channel (so `route_key`'s `Scheduled => Unhandled` arm stays as-is and no `Scheduled` events are emitted — that variant remains defined for completeness/forward-compat).

- [ ] **Step 3: Build + test**

Run: `cargo test --workspace && cargo build`
Expected: PASS. Then `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt --all -- --check` clean (fmt first if needed).

Manual smoke check (document, don't block on Ollama):
```bash
# With a [schedule] table (daily_time near the current time) in construct.toml,
# and a note edited "yesterday", run `entertheconstruct` and confirm a
# journal/<Y>/<M>/<DD>.md day note appears with the four sections, and a
# schedule_state row records last_run. For an immediate test, set daily_time to a
# minute or two from now (catch-up also fires it on first launch since last_run is empty).
```

- [ ] **Step 4: Commit**

```bash
git add crates/construct-cli/src/tui/watch_loop.rs
git -c user.name=Matt -c user.email=matt@matthewlittlehale.com commit -m "feat(cli): wire DailySummary orchestrator + scheduler poll (due + catch-up)" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Final verification (after all tasks)

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

All green. Confirm: existing tests still pass (reconcile skip + new helpers are additive); the daily-summary integration test passes; the watch loop builds the DailySummary orchestrator only when `[schedule]` is present.

## Self-Review checklist (done at plan-write time)
- **Spec coverage:** fire at daily_time + catch-up ✅ (Task 7 uses Plan 1 `due`; first tick evaluates persisted `last_run`); scan previous-day-mtime notes excl. journal/managed ✅ (Task 4 `changed_notes_on` + `is_excluded`); ensure `journal/YYYY/MM/` + zero-padded `DD.md` ✅ (Task 2 `journal_day_path` + Task 6 `create_dir_all`); four sections ✅ (Task 3 `render_day_note`); Today's Task List = open checkboxes from yesterday's notes + carryover, de-duped ✅ (Tasks 2/3/6); Carryover from yesterday's journal day-note ✅ (Task 6 reads `journal_day_path(yesterday)`); Yesterday summary = LLM prose gated ✅ (Task 6 `run_loop`+`validate_summary`); Other notes = links not otherwise captured ✅ (Task 6 `contributing` set + wikilinks); deterministic tasks vs agentic prose split ✅; journal loop-guarded ✅; idempotent re-run ✅ (Task 3 managed blocks + test); missing yesterday note → empty carryover ✅ (Task 6 `Err(_) => Vec::new()`); all local time ✅.
- **Type consistency:** `walk_notes`, `journal_day_path`, `scrape_open_checkboxes`, `dedupe_preserving_order`, `render_day_note`, `changed_notes_on`, `parse_hhmm`, `run_daily_summary(NaiveDate)` used consistently. `run_daily_summary` is `pub` and called directly by the scheduler task.
- **No placeholders:** every code step has real code. The one scoping decision (journal-folder constant vs struct field) is called out explicitly with the chosen path.
- **Reuse:** `run_loop`, `validate_summary`, `upsert_named`, `is_excluded`, `walk_notes`, `Store`, `schedule::due`/`last_firing_at_or_before`, `get/set_last_run`, `Clock` — all reused.
- **Configurable journal folder:** `run_daily_summary` takes `journal_folder: &str` (passed by the scheduler from `cfg.journal.folder`, default `"journal"`), so a custom `[journal] folder` is honored for both the changed-notes scan exclusion and the day-note path — no `Orchestrator` struct change.
