# Slice 4 "Release-Ready" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the Daily Briefs integration, journal tagging, inbox index table, carryover fix, robust recap, interactive setup wizard, and a ratatui dashboard — making The Construct installable and runnable by a stranger.

**Architecture:** Every feature follows existing patterns: pure helpers in `construct-engine/src/pipelines/`, watcher events in `construct-obsidian`, orchestrator methods for pipelines, SQLite state via the `Store` trait, CLI in `construct-cli`. Deterministic logic is pure-function-first and unit-tested; agentic calls go through the existing `run_loop` + gate validation.

**Tech Stack:** Rust (workspace), tokio, notify-debouncer-full, ratatui 0.28 + crossterm 0.28, sqlx/SQLite, chrono, serde_yaml frontmatter. New deps: `sha2` (brief hashing), `dotenvy` (.env loading), `dialoguer` (setup prompts).

**Spec:** `docs/superpowers/specs/2026-06-09-the-construct-slice-4-release-ready-design.md`

**Conventions for every task:**
- Run tests with `cargo test -p <crate>` from the repo root `/Users/matthewlittlehale/Sites/theconstruct`.
- Commit after every green task. Before the final task run `cargo fmt --all && cargo clippy --workspace --all-targets`.
- All file paths below are relative to the repo root.

---

## Phase 1 — Deterministic fixes

### Task 0: Branch

- [ ] **Step 1: Create the working branch**

```bash
git checkout -b slice-4-release-ready
```

(Implementation builds on `prod-readiness`, which holds the audit fixes this slice depends on.)

---

### Task 1: Carryover dedupe fix

The bug: `run_daily_summary` merges yesterday's open checkboxes into today's `daily-tasks` block AND renders them again in `daily-carryover` — the same task appears twice in today's note, and tomorrow's scrape re-collects from both blocks.

**Files:**
- Modify: `crates/construct-engine/src/pipelines/daily.rs`
- Modify: `crates/construct-engine/src/orchestrator.rs` (lines ~752–776, the task-merge section of `run_daily_summary`)

- [ ] **Step 1: Write failing tests in `daily.rs`'s `mod tests`**

```rust
#[test]
fn normalize_task_strips_checkbox_and_collapses_whitespace() {
    assert_eq!(normalize_task("- [ ]  buy   milk "), "buy milk");
    assert_eq!(normalize_task("buy milk"), "buy milk");
    assert_eq!(normalize_task("  call  Sam  "), "call Sam");
}

#[test]
fn dedupe_normalized_keeps_first_original_text() {
    let v = vec![
        "buy  milk".to_string(),
        "buy milk".to_string(),
        "call Sam".to_string(),
    ];
    assert_eq!(dedupe_normalized(v), vec!["buy  milk", "call Sam"]);
}

#[test]
fn partition_carryover_excludes_tasks_already_in_today() {
    let today = vec!["buy milk".to_string(), "ship release".to_string()];
    let yesterday = vec![
        "buy  milk".to_string(),   // already in today (normalized match) → excluded
        "old task".to_string(),    // genuinely open → carried
        "old task".to_string(),    // duplicate within yesterday → once
    ];
    assert_eq!(partition_carryover(&today, &yesterday), vec!["old task"]);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p construct-engine normalize_task`
Expected: FAIL — `cannot find function normalize_task`

- [ ] **Step 3: Implement in `daily.rs` (replace `dedupe_preserving_order` and its test — it has no other callers)**

```rust
/// Canonical form of a task line for duplicate detection: strip any leading
/// checkbox syntax, trim, collapse internal whitespace. Case is preserved —
/// "Email Bob" and "email bob" may be different tasks.
pub fn normalize_task(s: &str) -> String {
    let t = s.trim_start();
    let t = t.strip_prefix("- [ ]").or_else(|| t.strip_prefix("- [x]")).unwrap_or(t);
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// De-duplicate by normalized form, keeping the first occurrence's original text.
pub fn dedupe_normalized(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|i| seen.insert(normalize_task(i)))
        .collect()
}

/// Carryover = yesterday's open tasks NOT already present (normalized) in
/// today's task list, de-duplicated. This is what stops carryover compounding:
/// a task lives in exactly one section of today's note.
pub fn partition_carryover(today_tasks: &[String], yesterday_open: &[String]) -> Vec<String> {
    let today: std::collections::HashSet<String> =
        today_tasks.iter().map(|t| normalize_task(t)).collect();
    let mut seen = std::collections::HashSet::new();
    yesterday_open
        .iter()
        .filter(|t| {
            let n = normalize_task(t);
            !today.contains(&n) && seen.insert(n)
        })
        .cloned()
        .collect()
}
```

- [ ] **Step 4: Update the merge in `run_daily_summary` (orchestrator.rs, currently lines 766–776)**

Replace:

```rust
        let carryover = match std::fs::read_to_string(&yest_journal) {
            Ok(t) => daily::scrape_open_checkboxes(&t),
            Err(_) => Vec::new(),
        };
        let mut all_tasks = note_tasks.clone();
        all_tasks.extend(carryover.clone());
        let today_tasks = daily::dedupe_preserving_order(all_tasks);
```

with:

```rust
        let yesterday_open = match std::fs::read_to_string(&yest_journal) {
            Ok(t) => daily::scrape_open_checkboxes(&t),
            Err(_) => Vec::new(), // missing yesterday note → empty carryover, not an error
        };
        // Tasks block = tasks from yesterday's changed notes only; carryover block =
        // yesterday's still-open journal items not already in the task list. A task
        // appears in exactly ONE section, so tomorrow's scrape can't double-count it.
        let today_tasks = daily::dedupe_normalized(note_tasks.clone());
        let carryover = daily::partition_carryover(&today_tasks, &yesterday_open);
```

- [ ] **Step 5: Run the full engine test suite**

Run: `cargo test -p construct-engine`
Expected: PASS (orchestrator daily tests still pass — they assert section presence, not the old merge)

- [ ] **Step 6: Commit**

```bash
git add crates/construct-engine
git commit -m "fix(daily): carryover no longer duplicates tasks across sections or days"
```

---

### Task 2: Journal tag helper

**Files:**
- Create: `crates/construct-engine/src/pipelines/journal_tag.rs`
- Modify: `crates/construct-engine/src/pipelines/mod.rs` (add `pub mod journal_tag;`)

- [ ] **Step 1: Write the failing tests (inside the new file's `mod tests`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 9).unwrap()
    }

    #[test]
    fn tag_format_is_zero_padded() {
        assert_eq!(journal_tag(d()), "journal/2026/06/09");
    }

    #[test]
    fn adds_frontmatter_tag_and_trailing_literal() {
        let out = ensure_journal_tag("Some body text.", d());
        assert!(out.contains("journal/2026/06/09")); // frontmatter
        assert!(out.trim_end().ends_with("#journal/2026/06/09")); // body bottom
        assert!(out.starts_with("---\n")); // frontmatter was created
    }

    #[test]
    fn is_idempotent() {
        let once = ensure_journal_tag("Some body.", d());
        let twice = ensure_journal_tag(&once, d());
        assert_eq!(once, twice);
        assert_eq!(twice.matches("#journal/2026/06/09").count(), 1);
    }

    #[test]
    fn preserves_existing_frontmatter_and_tags() {
        let text = "---\ntags:\n- existing\n---\nBody.";
        let out = ensure_journal_tag(text, d());
        assert!(out.contains("existing"));
        assert!(out.contains("journal/2026/06/09"));
    }

    #[test]
    fn empty_body_gets_just_the_literal() {
        let out = ensure_journal_tag("", d());
        assert!(out.trim_end().ends_with("#journal/2026/06/09"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p construct-engine journal_tag`
Expected: FAIL — module does not exist

- [ ] **Step 3: Implement `journal_tag.rs`**

```rust
//! Journal date tagging: every note The Construct writes gets a
//! `journal/YYYY/MM/DD` frontmatter tag plus a literal `#journal/YYYY/MM/DD`
//! at the bottom of the body. Idempotent — re-applying is a no-op, so engine
//! re-writes never churn files (and the watcher only reacts to its configured
//! trigger tags, so journal tags can never start a feedback loop).
use chrono::{Datelike, NaiveDate};
use construct_obsidian::frontmatter::Note;

/// The vault tag for a calendar day: `journal/YYYY/MM/DD`, zero-padded.
pub fn journal_tag(date: NaiveDate) -> String {
    format!(
        "journal/{:04}/{:02}/{:02}",
        date.year(),
        date.month(),
        date.day()
    )
}

/// Idempotently apply the day tag to a note's full text. Adds to frontmatter
/// `tags:` (creating frontmatter if absent) and appends the literal `#tag` as
/// the last body line if it is not already present anywhere in the body.
pub fn ensure_journal_tag(text: &str, date: NaiveDate) -> String {
    let tag = journal_tag(date);
    let mut note = Note::parse(text);
    note.merge_tags(&[tag.clone()]);
    let literal = format!("#{tag}");
    let already_inline = note
        .body
        .split_whitespace()
        .any(|tok| tok.trim_end_matches(|c: char| !c.is_alphanumeric()) == literal || tok == literal);
    if !already_inline {
        let body = note.body.trim_end();
        note.body = if body.is_empty() {
            format!("{literal}\n")
        } else {
            format!("{body}\n\n{literal}\n")
        };
    }
    note.to_string()
}
```

- [ ] **Step 4: Register the module** — in `crates/construct-engine/src/pipelines/mod.rs` add `pub mod journal_tag;` alongside the existing `pub mod daily;` lines.

- [ ] **Step 5: Run tests**

Run: `cargo test -p construct-engine journal_tag`
Expected: PASS (5 tests)

- [ ] **Step 6: Commit**

```bash
git add crates/construct-engine
git commit -m "feat(engine): idempotent journal/YYYY/MM/DD tagging helper"
```

---

### Task 3: Wire journal tags into the write sites

**Files:**
- Modify: `crates/construct-engine/src/orchestrator.rs` — three sites: `run_inbox` (both branches), `run_tag`, `run_daily_summary`

- [ ] **Step 1: `run_inbox` move branch (currently ~line 588)** — after `note.remove(crate::pipelines::RUN_KEY);`, change:

```rust
            let finalized = note.to_string();
```

to:

```rust
            let finalized = crate::pipelines::journal_tag::ensure_journal_tag(
                &note.to_string(),
                chrono::Local::now().date_naive(),
            );
```

- [ ] **Step 2: `run_inbox` recommend branch (currently ~line 611)** — change:

```rust
            let recommended = inbox::apply_recommendation(
                &note.to_string(),
                &proposal.destination,
                &proposal.reason,
            );
```

to:

```rust
            let recommended = crate::pipelines::journal_tag::ensure_journal_tag(
                &inbox::apply_recommendation(
                    &note.to_string(),
                    &proposal.destination,
                    &proposal.reason,
                ),
                chrono::Local::now().date_naive(),
            );
```

- [ ] **Step 3: `run_tag` (currently ~line 687)** — change:

```rust
        write_atomic(
            path,
            &crate::pipelines::tag::apply_tags(&current, &tags, self.done_tag.as_deref()),
        )?;
```

to:

```rust
        write_atomic(
            path,
            &crate::pipelines::journal_tag::ensure_journal_tag(
                &crate::pipelines::tag::apply_tags(&current, &tags, self.done_tag.as_deref()),
                chrono::Local::now().date_naive(),
            ),
        )?;
```

- [ ] **Step 4: `run_daily_summary` (currently ~line 837)** — the day note gets its OWN date, not the processing date. Change:

```rust
        let updated =
            daily::render_day_note(&current, &today_tasks, &carryover, &prose, &other_links);
        write_atomic(&day_note, &updated)?;
```

to:

```rust
        let updated =
            daily::render_day_note(&current, &today_tasks, &carryover, &prose, &other_links);
        let updated = crate::pipelines::journal_tag::ensure_journal_tag(&updated, today);
        write_atomic(&day_note, &updated)?;
```

- [ ] **Step 5: Extend an existing orchestrator test** — in `orchestrator.rs` tests there is a daily-summary integration test (search `run_daily_summary` in `mod tests`). After its existing assertions on the day-note content, add:

```rust
        // Slice 4: day note carries its own journal date tag (frontmatter + literal).
        assert!(day_text.contains("journal/"), "frontmatter tag missing");
        assert!(day_text.contains("#journal/"), "literal tag missing");
```

(Adjust the variable name to whatever the test reads the day note into.)

- [ ] **Step 6: Run tests**

Run: `cargo test -p construct-engine`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/construct-engine
git commit -m "feat(engine): journal-tag every engine write (inbox, tag, day note)"
```

---

### Task 4: Inbox index as a table

**Files:**
- Modify: `crates/construct-engine/src/pipelines/inbox.rs` (`update_index`, `line_is_for_note`, tests)
- Modify: `crates/construct-engine/src/orchestrator.rs` (`run_inbox` step 5, ~line 631)

- [ ] **Step 1: Write failing tests (replace the existing `update_index_appends_and_dedupes_by_note` test)**

```rust
    #[test]
    fn update_index_renders_table_with_wikilink_to_new_location() {
        let e = IndexEntry {
            note_name: "idea.md",
            outcome: "moved",
            destination: Some("Reading"),
            when: "2026-06-09",
        };
        let idx = update_index("", &e);
        assert!(idx.contains("<!-- construct:inbox-log:start -->"));
        assert!(idx.contains("| Note | Outcome | Destination | When |"));
        assert!(idx.contains("| [[Reading/idea\\|idea]] | moved | Reading | 2026-06-09 |"));
    }

    #[test]
    fn update_index_dedupes_by_note_and_preserves_others() {
        let a = IndexEntry { note_name: "idea.md", outcome: "recommended", destination: None, when: "2026-06-08" };
        let idx = update_index("", &a);
        // Recommended note stays in Inbox → bare wikilink by name.
        assert!(idx.contains("| [[idea]] | recommended | — | 2026-06-08 |"));

        let b = IndexEntry { note_name: "todo.md", outcome: "moved", destination: Some("Projects"), when: "2026-06-09" };
        let idx = update_index(&idx, &b);

        // Re-processing idea.md replaces its row.
        let a2 = IndexEntry { note_name: "idea.md", outcome: "moved", destination: Some("Archive"), when: "2026-06-09" };
        let idx = update_index(&idx, &a2);
        assert_eq!(idx.matches("[[Archive/idea\\|idea]]").count(), 1);
        assert!(!idx.contains("recommended"));
        assert!(idx.contains("Projects")); // other rows preserved
        // Exactly one header.
        assert_eq!(idx.matches("| Note | Outcome |").count(), 1);
    }

    #[test]
    fn update_index_migrates_legacy_bullet_lines() {
        let legacy = "<!-- construct:inbox-log:start -->\n- `old.md` — moved→Reading\n<!-- construct:inbox-log:end -->";
        let e = IndexEntry { note_name: "new.md", outcome: "moved", destination: Some("Work"), when: "2026-06-09" };
        let idx = update_index(legacy, &e);
        // Legacy line becomes a row (outcome preserved verbatim, unknown columns dashed).
        assert!(idx.contains("| [[old]] | moved→Reading | — | — |"));
        assert!(idx.contains("| [[Work/new\\|new]] | moved | Work | 2026-06-09 |"));
        assert!(!idx.contains("- `old.md`"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p construct-engine update_index`
Expected: FAIL — `IndexEntry` not found

- [ ] **Step 3: Replace `update_index` + `line_is_for_note` in `inbox.rs`**

```rust
/// One processed-note record for the Inbox `_index.md` table.
pub struct IndexEntry<'a> {
    /// File name including `.md`, e.g. `idea.md`.
    pub note_name: &'a str,
    /// Short outcome, e.g. `moved` / `recommended`.
    pub outcome: &'a str,
    /// Vault-relative destination folder when the note was moved; `None` when
    /// it stayed in the Inbox (recommendation).
    pub destination: Option<&'a str>,
    /// ISO date the note was processed, e.g. `2026-06-09`.
    pub when: &'a str,
}

const TABLE_HEADER: &str = "| Note | Outcome | Destination | When |\n|---|---|---|---|";

/// Maintain the `inbox-log` managed block in `Inbox/_index.md` as a markdown
/// table. One row per processed note, keyed by note stem so re-processing
/// updates the row in place. Legacy bullet lines (pre-Slice-4) are migrated to
/// rows on first touch. `index_text` is the full current text of `_index.md`
/// ("" if it does not exist yet).
pub fn update_index(index_text: &str, entry: &IndexEntry) -> String {
    let existing = read_named(index_text, INDEX_BLOCK).unwrap_or_default();
    let stem = entry.note_name.strip_suffix(".md").unwrap_or(entry.note_name);

    let mut rows: Vec<String> = existing
        .lines()
        .filter_map(migrate_or_keep_row)
        .filter(|row| !row_is_for_note(row, stem))
        .collect();
    rows.push(render_row(stem, entry));

    let body = format!("{TABLE_HEADER}\n{}", rows.join("\n"));
    upsert_named(index_text, INDEX_BLOCK, &body)
}

/// Keep an existing table row as-is; convert a legacy `- \`name.md\` — outcome`
/// bullet into a row; drop header/blank lines (the header is re-rendered).
fn migrate_or_keep_row(line: &str) -> Option<String> {
    let t = line.trim();
    if t.is_empty() || t.starts_with("| Note ") || t.starts_with("|---") {
        return None;
    }
    if t.starts_with('|') {
        return Some(t.to_string());
    }
    // Legacy bullet: - `idea.md` — <outcome>
    let rest = t.strip_prefix("- `")?;
    let (name, after) = rest.split_once('`')?;
    let outcome = after.trim_start_matches([' ', '—', '-']).trim();
    let stem = name.strip_suffix(".md").unwrap_or(name);
    Some(format!("| [[{stem}]] | {outcome} | — | — |"))
}

/// A row belongs to a note if its first cell's wikilink targets the note stem
/// (either `[[stem]]` or `[[path/stem\|stem]]`).
fn row_is_for_note(row: &str, stem: &str) -> bool {
    let first_cell = row.trim_start_matches('|').split('|').next().unwrap_or("");
    first_cell.contains(&format!("[[{stem}]]"))
        || first_cell.contains(&format!("/{stem}\\|"))
        || first_cell.contains(&format!("\\|{stem}]]"))
}

fn render_row(stem: &str, entry: &IndexEntry) -> String {
    let link = match entry.destination {
        // Moved: link the note's NEW location so the link works post-move.
        Some(dest) => format!("[[{dest}/{stem}\\|{stem}]]"),
        // Still in Inbox: bare name link resolves wherever the note is.
        None => format!("[[{stem}]]"),
    };
    let dest = entry.destination.unwrap_or("—");
    format!("| {link} | {} | {dest} | {} |", entry.outcome, entry.when)
}
```

Keep the existing `use construct_obsidian::block::{read_named, upsert_named, upsert_named_at_top};` import — it already covers what this needs. Delete the old `line_is_for_note`.

- [ ] **Step 4: Update the call site in `run_inbox` (orchestrator.rs ~line 631)** — replace:

```rust
        let summary_outcome = format!(
            "enriched {} url(s), summarized, tagged, {outcome}",
            link_lines.len()
        );
        if let Some(dir) = path.parent() {
            let index_path = dir.join("_index.md");
            let cur = std::fs::read_to_string(&index_path).unwrap_or_default();
            let updated = inbox::update_index(&cur, &note_name, &summary_outcome);
```

with:

```rust
        let moved_dest = is_existing.then(|| proposal.destination.clone());
        let when = chrono::Local::now().date_naive().to_string();
        let summary_outcome = format!(
            "enriched {} url(s), summarized, tagged, {outcome}",
            link_lines.len()
        );
        if let Some(dir) = path.parent() {
            let index_path = dir.join("_index.md");
            let cur = std::fs::read_to_string(&index_path).unwrap_or_default();
            let updated = inbox::update_index(
                &cur,
                &inbox::IndexEntry {
                    note_name: &note_name,
                    outcome: &summary_outcome,
                    destination: moved_dest.as_deref(),
                    when: &when,
                },
            );
```

(`is_existing` and `proposal` are in scope from the move-decision step above; `outcome` is unchanged.)

- [ ] **Step 5: Run tests**

Run: `cargo test -p construct-engine`
Expected: PASS — including the orchestrator inbox integration tests (they assert `_index.md` mentions the note name; table rows still contain it)

- [ ] **Step 6: Commit**

```bash
git add crates/construct-engine
git commit -m "feat(inbox): _index log becomes a table with links to moved notes"
```

---

## Phase 2 — Daily Briefs pipeline

### Task 5: `[briefs]` config section

**Files:**
- Modify: `crates/construct-config/src/lib.rs`
- Modify: `crates/construct-cli/src/commands.rs` (`SAMPLE_CONFIG` + `ConfigCheck` output)

- [ ] **Step 1: Write failing tests in `construct-config`'s `mod tests`**

```rust
    #[test]
    fn briefs_off_when_table_absent() {
        let cfg: Config = toml::from_str(sample()).unwrap();
        assert!(cfg.briefs.is_none());
    }

    #[test]
    fn briefs_defaults_folder() {
        let toml = format!("{}\n[briefs]\n", sample());
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.briefs.unwrap().folder, "AI/DailyBriefs");
    }

    #[test]
    fn rejects_empty_briefs_folder() {
        let toml = format!("{}\n[briefs]\nfolder = \"\"\n", sample());
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p construct-config briefs`
Expected: FAIL — no field `briefs`

- [ ] **Step 3: Implement** — add to the `Config` struct (after `schedule`):

```rust
    #[serde(default)]
    pub briefs: Option<BriefsCfg>,
```

Add alongside the other cfg structs:

```rust
/// Watch a vault folder of externally-written Daily Briefs and fold each
/// day's brief into that day's journal note.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct BriefsCfg {
    #[serde(default = "default_briefs_folder")]
    pub folder: String,
}
fn default_briefs_folder() -> String {
    "AI/DailyBriefs".to_string()
}
```

Add to `validate()` (after the schedule check):

```rust
        if let Some(briefs) = &self.briefs {
            if briefs.folder.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "briefs.folder must not be empty".into(),
                ));
            }
        }
```

- [ ] **Step 4: Document in `SAMPLE_CONFIG`** (commands.rs, after the `[schedule]` comment block):

```toml
# Fold externally-written Daily Briefs (filenames containing YYYY-MM-DD) into
# the matching journal day note, and feed them to the daily recap.
# [briefs]
# folder = "AI/DailyBriefs"
```

And in the `ConfigCheck` arm, after the schedule line:

```rust
            match &cfg.briefs {
                Some(b) => println!("  briefs:   on (folder={})", b.folder),
                None => println!("  briefs:   off"),
            }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p construct-config && cargo test -p construct-cli`
Expected: PASS (including `sample_config_is_valid`)

- [ ] **Step 6: Commit**

```bash
git add crates/construct-config crates/construct-cli
git commit -m "feat(config): optional [briefs] section (default AI/DailyBriefs)"
```

---

### Task 6: Watcher emits `BriefChanged`

**Files:**
- Modify: `crates/construct-obsidian/src/watcher.rs`
- Modify: `crates/construct-obsidian/Cargo.toml` (ensure `chrono.workspace = true` is in `[dependencies]` — add if absent)
- Modify: `crates/construct-engine/src/triggers/mod.rs` (`TriggerEvent::Brief` + `From`)
- Modify: `crates/construct-cli/src/tui/watch_loop.rs` (pass briefs dir into `watch`; route stub)

- [ ] **Step 1: Write failing tests in `watcher.rs`'s `mod tests`**

```rust
    use chrono::NaiveDate;

    #[test]
    fn parses_brief_date_from_filename() {
        let d = NaiveDate::from_ymd_opt(2026, 6, 9).unwrap();
        assert_eq!(parse_brief_date("2026-06-09.md"), Some(d));
        assert_eq!(parse_brief_date("Daily Brief 2026-06-09.md"), Some(d));
        assert_eq!(parse_brief_date("brief-2026-06-09-v2.md"), Some(d));
        assert_eq!(parse_brief_date("notes.md"), None);
        assert_eq!(parse_brief_date("2026-13-40.md"), None); // invalid date
        assert_eq!(parse_brief_date("café-2026-06-09.md"), Some(d)); // multibyte safe
    }

    #[test]
    fn classifies_brief_paths_before_tag_logic() {
        let briefs_dir = Path::new("/v/AI/DailyBriefs");
        let ev = classify_path(
            Path::new("/v/AI/DailyBriefs/2026-06-09.md"),
            Some(briefs_dir),
        );
        assert_eq!(
            ev,
            Some(VaultEvent::BriefChanged {
                path: "/v/AI/DailyBriefs/2026-06-09.md".into(),
                date: NaiveDate::from_ymd_opt(2026, 6, 9).unwrap(),
            })
        );
        // Dateless file in the briefs folder → ignored entirely.
        assert_eq!(
            classify_path(Path::new("/v/AI/DailyBriefs/readme.md"), Some(briefs_dir)),
            None
        );
        // Outside the briefs folder → not a brief.
        assert_eq!(classify_path(Path::new("/v/Notes/x.md"), Some(briefs_dir)), None);
        // No briefs folder configured → never a brief.
        assert_eq!(classify_path(Path::new("/v/AI/DailyBriefs/2026-06-09.md"), None), None);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p construct-obsidian brief`
Expected: FAIL — `parse_brief_date` not found

- [ ] **Step 3: Implement in `watcher.rs`**

Add the variant to `VaultEvent`:

```rust
    /// A Daily Brief file (in the configured briefs folder, dated filename)
    /// was created or modified.
    BriefChanged {
        path: PathBuf,
        date: chrono::NaiveDate,
    },
```

Add the pure helpers:

```rust
/// Find the first `YYYY-MM-DD` substring in a filename and parse it as a
/// calendar date. Returns None when no valid date is present.
pub fn parse_brief_date(file_name: &str) -> Option<chrono::NaiveDate> {
    let bytes = file_name.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    for i in 0..=bytes.len() - 10 {
        let Ok(window) = std::str::from_utf8(&bytes[i..i + 10]) else {
            continue; // window splits a multibyte char — not a date
        };
        if let Ok(d) = chrono::NaiveDate::parse_from_str(window, "%Y-%m-%d") {
            return Some(d);
        }
    }
    None
}

/// Path-based classification that runs BEFORE content classification: a `.md`
/// file under the briefs dir with a dated filename is a brief event; any other
/// file under the briefs dir is ignored (the folder is engine-input, not notes).
pub fn classify_path(path: &Path, briefs_dir: Option<&Path>) -> Option<VaultEvent> {
    let dir = briefs_dir?;
    if !path.starts_with(dir) {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    let date = parse_brief_date(name)?;
    Some(VaultEvent::BriefChanged {
        path: path.to_path_buf(),
        date,
    })
}
```

Change `watch()` to take and use the briefs dir — new signature and closure body:

```rust
pub fn watch(
    vault: PathBuf,
    known_tags: Vec<String>,
    status_key: String,
    briefs_dir: Option<PathBuf>,
    tx: mpsc::UnboundedSender<VaultEvent>,
) -> notify_debouncer_full::Debouncer<notify::RecommendedWatcher, notify_debouncer_full::FileIdMap>
```

and inside the event loop, replace the per-path body with:

```rust
                for path in &ev.paths {
                    if path.extension().and_then(|e| e.to_str()) != Some("md") {
                        continue;
                    }
                    // Briefs are classified by PATH and routed without reading
                    // tags — and files under the briefs dir never fall through
                    // to tag classification.
                    if let Some(dir) = &briefs_dir {
                        if path.starts_with(dir) {
                            if let Some(ev) = classify_path(path, Some(dir)) {
                                let _ = tx.send(ev);
                            }
                            continue;
                        }
                    }
                    let Ok(text) = std::fs::read_to_string(path) else {
                        continue;
                    };
                    if let Some(vault_event) = classify(path, &text, &known_tags, &status_key) {
                        let _ = tx.send(vault_event);
                    }
                }
```

- [ ] **Step 4: Extend `TriggerEvent` (triggers/mod.rs)**

```rust
    /// A Daily Brief file changed (Slice 4).
    Brief {
        path: PathBuf,
        date: chrono::NaiveDate,
    },
```

and in `impl From<VaultEvent>` add:

```rust
            VaultEvent::BriefChanged { path, date } => TriggerEvent::Brief { path, date },
```

- [ ] **Step 5: Fix the `watch()` call site** — in `watch_loop.rs` (~line 267):

```rust
    let briefs_dir: Option<PathBuf> = cfg
        .briefs
        .as_ref()
        .map(|b| PathBuf::from(shellexpand_tilde(&cfg.vault.path)).join(&b.folder));
    let _debouncer = watch(
        vault.into(),
        known_tags(&cfg),
        "construct_status".into(),
        briefs_dir,
        tx_vault,
    );
```

And add a temporary routing arm so it compiles (full routing lands in Task 8): in `route_key`, add

```rust
        TriggerEvent::Brief { .. } => RouteTarget::Unhandled,
```

- [ ] **Step 6: Run the workspace build + tests**

Run: `cargo test --workspace`
Expected: PASS (chrono must be in construct-obsidian's deps; add `chrono.workspace = true` if the build complains)

- [ ] **Step 7: Commit**

```bash
git add crates/construct-obsidian crates/construct-engine crates/construct-cli
git commit -m "feat(watcher): classify Daily Brief files into BriefChanged events"
```

---

### Task 7: Brief hash state in the store

**Files:**
- Create: `crates/construct-store/migrations/0003_brief_state.sql`
- Modify: `crates/construct-core/src/store.rs` (trait methods)
- Modify: `crates/construct-store/src/lib.rs` (SQLite impl + test)
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`: add `sha2 = "0.10"`)
- Modify: `crates/construct-engine/Cargo.toml` (add `sha2.workspace = true`)

Check first: `grep -rn "impl Store for" crates/` — if anything besides `SqliteStore` implements the trait (e.g. a test mock in `construct-engine/src/testkit.rs`), give it the same two methods returning `Ok(None)` / `Ok(())`.

- [ ] **Step 1: Migration file `0003_brief_state.sql`**

```sql
-- Content-hash guard for Daily Briefs: re-saving an unchanged brief must not
-- re-trigger the (token-spending) recap agent.
CREATE TABLE IF NOT EXISTS brief_state (
    path       TEXT PRIMARY KEY,
    hash       TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

- [ ] **Step 2: Trait methods in `construct-core/src/store.rs`** (after `set_last_run`):

```rust
    /// Last-processed content hash for a Daily Brief file, by absolute path.
    async fn get_brief_hash(&self, path: &str) -> Result<Option<String>, StoreError>;
    /// Record the content hash after successfully processing a brief.
    async fn set_brief_hash(&self, path: &str, hash: &str) -> Result<(), StoreError>;
```

- [ ] **Step 3: SQLite impl in `construct-store/src/lib.rs`** (mirror `get_last_run`/`set_last_run`, ~line 181):

```rust
    async fn get_brief_hash(&self, path: &str) -> Result<Option<String>, StoreError> {
        let row = sqlx::query("SELECT hash FROM brief_state WHERE path = ?")
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_store_err)?;
        Ok(row.map(|r| r.get::<String, _>(0)))
    }

    async fn set_brief_hash(&self, path: &str, hash: &str) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO brief_state (path, hash, updated_at) VALUES (?, ?, datetime('now'))
             ON CONFLICT(path) DO UPDATE SET hash = excluded.hash, updated_at = datetime('now')",
        )
        .bind(path)
        .bind(hash)
        .execute(&self.pool)
        .await
        .map_err(to_store_err)?;
        Ok(())
    }
```

(Match the exact error-mapping helper name used by the neighboring methods — read them and copy the pattern; if they map errors inline, do the same.)

- [ ] **Step 4: Store test** (next to `schedule_state_roundtrips`):

```rust
    #[tokio::test]
    async fn brief_state_roundtrips_and_overwrites() {
        let store = mem_store().await; // same constructor the neighboring test uses
        assert_eq!(store.get_brief_hash("/v/b.md").await.unwrap(), None);
        store.set_brief_hash("/v/b.md", "abc").await.unwrap();
        assert_eq!(store.get_brief_hash("/v/b.md").await.unwrap(), Some("abc".into()));
        store.set_brief_hash("/v/b.md", "def").await.unwrap();
        assert_eq!(store.get_brief_hash("/v/b.md").await.unwrap(), Some("def".into()));
    }
```

(Use the same in-memory-store setup as `schedule_state_roundtrips` — read that test and reuse its connect line verbatim.)

- [ ] **Step 5: Run tests**

Run: `cargo test -p construct-store && cargo test --workspace`
Expected: PASS (workspace run catches any other `impl Store` you must extend)

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/construct-core crates/construct-store crates/construct-engine
git commit -m "feat(store): brief_state table + hash accessors for the brief guard"
```

---

### Task 8: `run_brief` pipeline + routing

**Files:**
- Create: `crates/construct-engine/src/pipelines/brief.rs`
- Modify: `crates/construct-engine/src/pipelines/mod.rs` (add `pub mod brief;`)
- Modify: `crates/construct-engine/src/orchestrator.rs` (new `run_brief` method)
- Modify: `crates/construct-cli/src/tui/watch_loop.rs` (route `Brief` events; build daily orchestrator when `[briefs]` OR `[schedule]` present)

- [ ] **Step 1: Write failing tests for the pure helpers (`brief.rs` `mod tests`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const BRIEF: &str = "# Daily Brief\n\nIntro sentence.\n\n## Calendar\n- 10:00 Standup\n- 14:00 1:1 with Ana\n\n## Email highlights\n- Contract signed\nSome prose here.\n- Follow up with vendor\n";

    #[test]
    fn outline_collects_headings_and_bullets_in_order() {
        let items = extract_outline(BRIEF, 10);
        assert_eq!(
            items,
            vec![
                "**Calendar**",
                "- 10:00 Standup",
                "- 14:00 1:1 with Ana",
                "**Email highlights**",
                "- Contract signed",
                "- Follow up with vendor",
            ]
        );
    }

    #[test]
    fn outline_caps_items_and_skips_top_level_title() {
        let items = extract_outline(BRIEF, 3);
        assert_eq!(items.len(), 3);
        assert!(!items.iter().any(|i| i.contains("Daily Brief")));
    }

    #[test]
    fn renders_section_with_wikilink_and_outline() {
        let s = render_brief_section("2026-06-09", &["**Calendar**".into(), "- 10:00 Standup".into()]);
        assert!(s.starts_with("## Daily Brief\n"));
        assert!(s.contains("[[2026-06-09]]"));
        assert!(s.contains("- 10:00 Standup"));
    }

    #[test]
    fn renders_link_only_when_outline_empty() {
        let s = render_brief_section("2026-06-09", &[]);
        assert!(s.contains("[[2026-06-09]]"));
    }

    #[test]
    fn content_hash_is_stable_and_distinguishes() {
        assert_eq!(content_hash("a"), content_hash("a"));
        assert_ne!(content_hash("a"), content_hash("b"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p construct-engine brief`
Expected: FAIL — module not found

- [ ] **Step 3: Implement `brief.rs`**

```rust
//! Daily Brief pure helpers: outline extraction, day-note section rendering,
//! and the content hash used to guard re-processing.
use sha2::{Digest, Sha256};

/// Hex SHA-256 of brief content. Stable across runs/platforms (unlike the
/// std hasher), so the guard survives restarts.
pub fn content_hash(text: &str) -> String {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    format!("{:x}", h.finalize())
}

/// Deterministic outline of a brief: `##+` headings (bolded) and top-level
/// `-`/`*` bullets, in document order, capped at `max_items`. The top-level
/// `#` title is skipped (it duplicates the section heading we render).
pub fn extract_outline(text: &str, max_items: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if out.len() >= max_items {
            break;
        }
        let t = line.trim_end();
        if let Some(h) = t.strip_prefix("## ").or_else(|| t.strip_prefix("### ")) {
            out.push(format!("**{}**", h.trim()));
        } else if (t.starts_with("- ") || t.starts_with("* ")) && !t.starts_with("- [") {
            out.push(format!("- {}", t[2..].trim()));
        }
    }
    out
}

/// Render the `daily-brief` managed-block body: heading, wikilink to the brief
/// note, and its outline.
pub fn render_brief_section(brief_stem: &str, outline: &[String]) -> String {
    let mut s = format!("## Daily Brief\n\n[[{brief_stem}]]\n");
    if !outline.is_empty() {
        s.push('\n');
        for item in outline {
            s.push_str(item);
            s.push('\n');
        }
    }
    s
}
```

Register `pub mod brief;` in `pipelines/mod.rs`.

- [ ] **Step 4: Add `Orchestrator::run_brief` (orchestrator.rs, after `run_daily_summary`)**

```rust
    /// Brief pipeline (event-driven). Hash-guarded: unchanged content is a
    /// no-op. Updates the `daily-brief` managed block in the brief's day note,
    /// then refreshes the recap so it can fold the brief in. Like the daily
    /// pipeline, this never writes frontmatter claims to the day note.
    pub async fn run_brief(
        &self,
        path: &Path,
        date: chrono::NaiveDate,
        journal_folder: &str,
    ) -> anyhow::Result<()> {
        use crate::pipelines::{brief, daily, journal_tag};

        let text = std::fs::read_to_string(path)?;
        let hash = brief::content_hash(&text);
        let path_str = path.to_string_lossy().to_string();
        if self.store.get_brief_hash(&path_str).await?.as_deref() == Some(hash.as_str()) {
            tracing::debug!("brief unchanged, skipping: {path_str}");
            return Ok(());
        }

        // 1. Deterministic: upsert the daily-brief block into the day note.
        let day_note = self.vault_path.join(daily::journal_day_path(journal_folder, date));
        if let Some(parent) = day_note.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let outline = brief::extract_outline(&text, 12);
        let section = brief::render_brief_section(&stem, &outline);
        let current = std::fs::read_to_string(&day_note).unwrap_or_default();
        let updated = construct_obsidian::block::upsert_named(&current, "daily-brief", &section);
        let updated = journal_tag::ensure_journal_tag(&updated, date);
        write_atomic(&day_note, &updated)?;

        // 2. Agentic: refresh the recap (it reads the brief as context). A
        //    recap failure must not lose the deterministic update above, and
        //    the hash is only recorded on full success so the next event retries.
        self.run_daily_summary(date, journal_folder).await?;
        self.store.set_brief_hash(&path_str, &hash).await?;
        tracing::info!("brief folded into day note: {stem} → {}", day_note.display());
        Ok(())
    }
```

- [ ] **Step 5: Route brief events (watch_loop.rs)**

In `RouteTarget`, add a `Daily` variant; in `route_key`, change the `Brief` arm to `RouteTarget::Daily`. Change the daily orchestrator condition (~line 208) from `if cfg.schedule.is_some()` to:

```rust
    let daily_orch: Option<Arc<Orchestrator>> = if cfg.schedule.is_some() || cfg.briefs.is_some() {
```

In the event loop `match route_key(&event)`, add:

```rust
            RouteTarget::Daily => {
                if let (Some(o), TriggerEvent::Brief { path, date }) = (daily_orch.as_ref(), event)
                {
                    let o = o.clone();
                    let journal_folder = journal_folder_for_briefs.clone();
                    tokio::spawn(async move {
                        if let Err(e) = o.run_brief(&path, date, &journal_folder).await {
                            tracing::error!("brief handler error: {e}");
                        }
                    });
                }
            }
```

Before the loop, capture the folder once (next to the other journal-folder lookups):

```rust
    let journal_folder_for_briefs = cfg
        .journal
        .as_ref()
        .map(|j| j.folder.clone())
        .unwrap_or_else(|| "journal".to_string());
```

Also extend the startup banner (after the `daily:` line):

```rust
    if let Some(b) = &cfg.briefs {
        println!("  briefs:  {}/ (event-driven)", b.folder);
    }
```

- [ ] **Step 6: Integration test for the hash guard** — in `orchestrator.rs` `mod tests`, next to the existing daily-summary test (reuse its store/orchestrator setup helper):

```rust
    #[tokio::test]
    async fn run_brief_updates_day_note_and_hash_guard_skips_rerun() {
        // Build orchestrator exactly like the daily-summary test does (ScriptedModel
        // returning one valid recap JSON), with a tempdir vault.
        // 1. Write vault/AI/DailyBriefs/2026-06-09.md with a heading + bullet.
        // 2. run_brief(path, 2026-06-09, "journal") → Ok.
        //    Assert journal/2026/06/09.md contains "construct:daily-brief:start",
        //    the wikilink "[[2026-06-09]]", and the bullet text.
        // 3. run_brief again with UNCHANGED content → Ok, and the ScriptedModel
        //    records no additional calls (hash guard short-circuited).
        // 4. Append a line to the brief; run_brief again → model called once more.
    }
```

Write this as real code following the established test-helper pattern in that module (`ScriptedModel`, `SqliteStore::connect("sqlite::memory:")`, tempdir vault). The comments above are the required assertions.

- [ ] **Step 7: Run tests**

Run: `cargo test --workspace`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add crates/construct-engine crates/construct-cli
git commit -m "feat(briefs): event-driven brief pipeline with content-hash guard"
```

---

## Phase 3 — Robust recap

### Task 9: Richer recap gate

**Files:**
- Modify: `crates/construct-engine/src/gate.rs`

- [ ] **Step 1: Failing tests (gate.rs `mod tests`)**

```rust
    #[test]
    fn validate_recap_accepts_full_shape() {
        let json = r#"{"tldr": "Busy day.", "highlights": ["Shipped X"], "action_items": ["Email Bob"]}"#;
        let r = validate_recap(json).unwrap();
        assert_eq!(r.tldr, "Busy day.");
        assert_eq!(r.highlights, vec!["Shipped X"]);
        assert_eq!(r.action_items, vec!["Email Bob"]);
    }

    #[test]
    fn validate_recap_defaults_missing_lists() {
        let r = validate_recap(r#"{"tldr": "Quiet."}"#).unwrap();
        assert!(r.highlights.is_empty());
        assert!(r.action_items.is_empty());
    }

    #[test]
    fn validate_recap_rejects_empty_tldr_and_garbage() {
        assert!(validate_recap(r#"{"tldr": ""}"#).is_err());
        assert!(validate_recap("not json").is_err());
    }

    #[test]
    fn validate_recap_tolerates_code_fences() {
        // Mirror whatever fence-stripping validate_summary does — reuse the
        // same pre-processing helper so behavior stays consistent.
        let fenced = "```json\n{\"tldr\": \"ok\"}\n```";
        assert!(validate_recap(fenced).is_ok());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p construct-engine validate_recap`
Expected: FAIL — not found

- [ ] **Step 3: Implement** — read `validate_summary` in `gate.rs` first and reuse its JSON-extraction/fence-stripping helper exactly. Then:

```rust
#[derive(Debug, serde::Deserialize)]
pub struct Recap {
    pub tldr: String,
    #[serde(default)]
    pub highlights: Vec<String>,
    #[serde(default)]
    pub action_items: Vec<String>,
}

/// Validate the daily-recap agent output: strict JSON with a non-empty tldr;
/// highlights/action_items optional. Same fence tolerance as validate_summary.
pub fn validate_recap(content: &str) -> Result<Recap, GateError> {
    let json = extract_json(content)?; // ← whatever helper validate_summary uses
    let recap: Recap =
        serde_json::from_str(&json).map_err(|e| GateError::Schema(e.to_string()))?;
    if recap.tldr.trim().is_empty() {
        return Err(GateError::Schema("tldr must be non-empty".into()));
    }
    Ok(recap)
}
```

(Adapt the helper name and `GateError` variant to what `gate.rs` actually defines — copy the neighboring function's structure.)

- [ ] **Step 4: Run, then commit**

Run: `cargo test -p construct-engine gate`
Expected: PASS

```bash
git add crates/construct-engine
git commit -m "feat(gate): validate_recap for the richer daily summary shape"
```

---

### Task 10: Recap inputs, template, and render

**Files:**
- Create: `prompts/daily_summary.md`
- Modify: `crates/construct-engine/src/pipelines/daily.rs` (excerpt + checked-scrape + template fill + summary render)
- Modify: `crates/construct-engine/src/orchestrator.rs` (`run_daily_summary` step 3; new `prompt_dir` field)
- Modify: `crates/construct-cli/src/tui/watch_loop.rs` (pass `prompt_dir` at all 3 `Orchestrator { … }` construction sites)
- Modify: `scripts/setup-home.sh` + `scripts/update.sh` — verify they deploy `prompts/*.md` wholesale (they do; if either lists files explicitly, add `daily_summary.md`)

- [ ] **Step 1: Failing tests in `daily.rs`**

```rust
    #[test]
    fn excerpt_strips_frontmatter_and_caps_at_line_boundary() {
        let text = "---\ntags: [x]\n---\nLine one.\nLine two is long.\nLine three.";
        let e = excerpt(text, 25);
        assert!(e.starts_with("Line one."));
        assert!(e.len() <= 27); // cap + ellipsis
        assert!(e.ends_with('…'));
        // Under the cap → whole body, no ellipsis.
        assert_eq!(excerpt("---\na: b\n---\nShort.", 100), "Short.");
    }

    #[test]
    fn scrape_checked_checkboxes_collects_done_items() {
        let text = "- [ ] open\n- [x] done one\n  - [x] nested done\n- [X] CAPS done";
        assert_eq!(
            scrape_checked_checkboxes(text),
            vec!["done one", "nested done", "CAPS done"]
        );
    }

    #[test]
    fn fill_recap_template_replaces_all_slots() {
        let t = "N:{{NOTE_EXCERPTS}} C:{{COMPLETED}} Y:{{CARRYOVER}} B:{{BRIEF}}";
        let out = fill_recap_template(t, "notes!", "done!", "carry!", "brief!");
        assert_eq!(out, "N:notes! C:done! Y:carry! B:brief!");
        assert!(!out.contains("{{"));
    }

    #[test]
    fn render_summary_section_includes_all_parts_as_plain_bullets() {
        let s = render_summary_section(
            "A solid day.",
            &["Shipped the release".into()],
            &["Email Bob".into()],
        );
        assert!(s.starts_with("## Yesterday summary"));
        assert!(s.contains("A solid day."));
        assert!(s.contains("**Highlights**"));
        assert!(s.contains("- Shipped the release"));
        assert!(s.contains("**Action items**"));
        // Plain bullets, NOT checkboxes — action items must not leak into
        // tomorrow's carryover scrape.
        assert!(s.contains("- Email Bob"));
        assert!(!s.contains("- [ ] Email Bob"));
    }

    #[test]
    fn render_summary_section_omits_empty_lists() {
        let s = render_summary_section("Quiet day.", &[], &[]);
        assert!(!s.contains("Highlights"));
        assert!(!s.contains("Action items"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p construct-engine excerpt`
Expected: FAIL

- [ ] **Step 3: Implement in `daily.rs`**

```rust
use construct_obsidian::frontmatter::Note;

/// Body-only excerpt for recap input: frontmatter stripped, cut at the last
/// full line within `max_chars`, ellipsis appended when truncated.
pub fn excerpt(text: &str, max_chars: usize) -> String {
    let body = Note::parse(text).body;
    let body = body.trim();
    if body.chars().count() <= max_chars {
        return body.to_string();
    }
    let mut taken = String::new();
    for line in body.lines() {
        if taken.chars().count() + line.chars().count() + 1 > max_chars {
            break;
        }
        if !taken.is_empty() {
            taken.push('\n');
        }
        taken.push_str(line);
    }
    if taken.is_empty() {
        // Single very long line: hard char cut.
        taken = body.chars().take(max_chars).collect();
    }
    format!("{taken}…")
}

/// Extract the text of every CHECKED checkbox (`- [x]` / `- [X]`), trimmed.
pub fn scrape_checked_checkboxes(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        let rest = t.strip_prefix("- [x]").or_else(|| t.strip_prefix("- [X]"));
        if let Some(rest) = rest {
            let task = rest.trim();
            if !task.is_empty() {
                out.push(task.to_string());
            }
        }
    }
    out
}

/// Default recap prompt, embedded so a missing prompts/ deploy can never
/// break the daily run. `prompts/daily_summary.md` overrides it when present.
pub const DEFAULT_RECAP_TEMPLATE: &str = include_str!("../../../../prompts/daily_summary.md");

/// Fill the recap template's four slots. Plain string replacement — the
/// template is trusted local config, not user input.
pub fn fill_recap_template(
    template: &str,
    note_excerpts: &str,
    completed: &str,
    carryover: &str,
    brief: &str,
) -> String {
    template
        .replace("{{NOTE_EXCERPTS}}", note_excerpts)
        .replace("{{COMPLETED}}", completed)
        .replace("{{CARRYOVER}}", carryover)
        .replace("{{BRIEF}}", brief)
}

/// Render the daily-summary managed-block body from a validated recap.
/// Action items are plain bullets on purpose: checkbox syntax here would be
/// scraped into tomorrow's carryover and double-count against the task list.
pub fn render_summary_section(tldr: &str, highlights: &[String], action_items: &[String]) -> String {
    let mut s = format!("## Yesterday summary\n\n{}\n", tldr.trim());
    if !highlights.is_empty() {
        s.push_str("\n**Highlights**\n");
        for h in highlights {
            s.push_str(&format!("- {h}\n"));
        }
    }
    if !action_items.is_empty() {
        s.push_str("\n**Action items**\n");
        for a in action_items {
            s.push_str(&format!("- {a}\n"));
        }
    }
    s
}
```

Then change `render_day_note` so the summary body is passed in pre-rendered: rename its `prose: &str` parameter to `summary_body: &str` and replace

```rust
    let summary_body = format!("## Yesterday summary\n\n{}", prose.trim());
```

with

```rust
    let summary_body = summary_body.trim_end().to_string();
```

Update `render_day_note`'s own test to pass `"## Yesterday summary\n\nYou edited two notes about the project."` for that argument.

- [ ] **Step 4: Create `prompts/daily_summary.md`**

```markdown
You are writing the "Yesterday summary" section of a personal daily journal
note. Be specific and concrete — name the actual notes, meetings, and tasks.
Do not invent anything that is not in the inputs. Write in second person
("you shipped…", "you met…").

INPUTS

Notes changed yesterday (excerpts):
{{NOTE_EXCERPTS}}

Tasks completed yesterday:
{{COMPLETED}}

Tasks still carried over:
{{CARRYOVER}}

Today's Daily Brief (calendar/email context, may be empty):
{{BRIEF}}

Write a recap with:
- "tldr": 3-6 sentences summarizing what actually happened and what it adds up to.
- "highlights": up to 5 short bullets of the most notable items.
- "action_items": up to 5 concrete next steps implied by the inputs.

Return STRICT JSON only: {"tldr": string, "highlights": [string], "action_items": [string]}
```

- [ ] **Step 5: Upgrade `run_daily_summary` (orchestrator.rs step 3, lines ~778–817)**

Add the field to `Orchestrator` (after `exclude_dirs`):

```rust
    /// Directory holding prompt templates (e.g. ~/.theconstruct/prompts);
    /// None → embedded defaults.
    pub prompt_dir: Option<std::path::PathBuf>,
```

Set it at all three construction sites in `watch_loop.rs` with `prompt_dir: Some(base_dir.join("prompts")),` and in every test orchestrator with `prompt_dir: None,`.

Replace the prose step:

```rust
        // 3. Rich recap (agentic) — skip the model entirely if nothing changed
        //    AND there is no brief to fold in.
        let brief_text = self.todays_brief_text(today);
        let summary_body = if changed.is_empty() && brief_text.is_empty() {
            daily::render_summary_section("No notes changed yesterday.", &[], &[])
        } else {
            // Inputs: per-note excerpts capped at 700 chars, 12 notes / ~8k chars total.
            let mut excerpts = String::new();
            for p in changed.iter().take(12) {
                if excerpts.chars().count() > 8_000 {
                    break;
                }
                if let Ok(text) = std::fs::read_to_string(p) {
                    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
                    excerpts.push_str(&format!("### {stem}\n{}\n\n", daily::excerpt(&text, 700)));
                }
            }
            let completed = daily::scrape_checked_checkboxes(
                &std::fs::read_to_string(&yest_journal).unwrap_or_default(),
            );
            let template = self.recap_template();
            let user_prompt = daily::fill_recap_template(
                &template,
                if excerpts.is_empty() { "(none)" } else { &excerpts },
                &bullet_list_or_none(&completed),
                &bullet_list_or_none(&carryover),
                if brief_text.is_empty() { "(none)" } else { &brief_text },
            );
            let messages = vec![
                ChatMessage::system(&self.system_prompt),
                ChatMessage::user(user_prompt),
            ];
            match run_loop(
                self.provider.as_ref(),
                &self.tools,
                messages,
                &LoopConfig {
                    model: self.model.clone(),
                    max_iterations: self.max_iterations,
                },
            )
            .await
            {
                Ok(r) => match crate::gate::validate_recap(&r.content) {
                    Ok(rec) => {
                        daily::render_summary_section(&rec.tldr, &rec.highlights, &rec.action_items)
                    }
                    Err(e) => return self.fail_daily(&run_id, &e.to_string()).await,
                },
                Err(e) => return self.fail_daily(&run_id, &e.to_string()).await,
            }
        };
```

(`render_day_note` is then called with `&summary_body` in place of `&prose`. A failed agent call still returns before any write — the previous `daily-summary` block is never blanked.)

Add the helpers as private `Orchestrator` methods + one free function:

```rust
    /// prompts/daily_summary.md from prompt_dir when present, else the
    /// embedded default — recap stays tunable without recompiling.
    fn recap_template(&self) -> String {
        self.prompt_dir
            .as_ref()
            .map(|d| d.join("daily_summary.md"))
            .filter(|p| p.exists())
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_else(|| crate::pipelines::daily::DEFAULT_RECAP_TEMPLATE.to_string())
    }

    /// Excerpt of today's Daily Brief ("" when briefs are off / no brief yet).
    fn todays_brief_text(&self, today: chrono::NaiveDate) -> String {
        let Some(folder) = &self.briefs_folder else {
            return String::new();
        };
        let dir = self.vault_path.join(folder);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return String::new();
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".md")
                && construct_obsidian::watcher::parse_brief_date(&name) == Some(today)
            {
                if let Ok(text) = std::fs::read_to_string(e.path()) {
                    return crate::pipelines::daily::excerpt(&text, 1_500);
                }
            }
        }
        String::new()
    }
```

```rust
/// "- item" lines, or "(none)" — keeps template slots non-empty and unambiguous.
fn bullet_list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items.iter().map(|i| format!("- {i}")).collect::<Vec<_>>().join("\n")
    }
}
```

This needs one more `Orchestrator` field (set alongside `prompt_dir` at all construction sites; `None` in tests except brief/daily tests that exercise it):

```rust
    /// Vault-relative Daily Briefs folder ([briefs].folder), None when off.
    pub briefs_folder: Option<String>,
```

In `watch_loop.rs` set it from config at all three sites: `briefs_folder: cfg.briefs.as_ref().map(|b| b.folder.clone()),`.

- [ ] **Step 6: Fix every test-orchestrator constructor** — `cargo test --workspace` will list each `Orchestrator { … }` literal missing the two new fields; add `prompt_dir: None, briefs_folder: None,` (or real values in the brief test from Task 8).

- [ ] **Step 7: Update the existing daily-summary integration test** — the `ScriptedModel`'s scripted response must now be the richer JSON, e.g. `{"tldr": "You worked on the project plan.", "highlights": ["Project plan"], "action_items": ["Review budget"]}`, and assertions extended:

```rust
        assert!(day_text.contains("**Highlights**"));
        assert!(day_text.contains("- Project plan"));
```

- [ ] **Step 8: Run, then commit**

Run: `cargo test --workspace`
Expected: PASS

```bash
git add prompts crates scripts
git commit -m "feat(daily): rich recap — excerpts, completed/carryover/brief inputs, external template"
```

---

## Phase 4 — Interactive setup

### Task 11: `.env` loading at startup

**Files:**
- Modify: `Cargo.toml` (workspace deps: `dotenvy = "0.15"`)
- Modify: `crates/construct-cli/Cargo.toml` (`dotenvy.workspace = true`)
- Modify: `crates/construct-cli/src/main.rs`

- [ ] **Step 1: Implement (no test harness for process env — verified by the wizard's integration test in Task 12)**

In `main.rs`, before the tracing init:

```rust
    // Load ~/.theconstruct/.env (or $CONSTRUCT_HOME/.env) so API keys written
    // by `entertheconstruct setup` are available without shell-profile exports.
    // Existing process env vars always win (dotenvy never overrides).
    if let Some(env_path) = construct_home_env_path() {
        if env_path.exists() {
            let _ = dotenvy::from_path(&env_path);
        }
    }
```

and at the bottom of the file:

```rust
/// $CONSTRUCT_HOME/.env, else ~/.theconstruct/.env. Mirrors the config-path
/// resolution in commands.rs.
fn construct_home_env_path() -> Option<std::path::PathBuf> {
    if let Some(home) = std::env::var_os("CONSTRUCT_HOME") {
        return Some(std::path::PathBuf::from(home).join(".env"));
    }
    std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".theconstruct").join(".env"))
}
```

- [ ] **Step 2: Build + commit**

Run: `cargo build -p construct-cli`
Expected: compiles clean

```bash
git add Cargo.toml Cargo.lock crates/construct-cli
git commit -m "feat(cli): load ~/.theconstruct/.env at startup"
```

---

### Task 12: `entertheconstruct setup` wizard

**Files:**
- Create: `crates/construct-cli/src/setup.rs`
- Modify: `crates/construct-cli/src/main.rs` (add `mod setup;`)
- Modify: `crates/construct-cli/src/commands.rs` (subcommand + dispatch)
- Modify: `Cargo.toml` (workspace deps: `dialoguer = "0.11"`)
- Modify: `crates/construct-cli/Cargo.toml` (`dialoguer.workspace = true`)
- Modify: `scripts/setup-home.sh` (hand off to the wizard)

Design rules (from the spec):
- **Never rewrites an existing `construct.toml`** — it validates it and reports; config generation happens only on first run. Hand-edits stay safe.
- Keys go to `<home>/.env` at mode `0600`; re-running shows which keys are already set (never echoes values) and lets you replace them.
- `--non-interactive` takes everything as flags — scriptable and testable.

- [ ] **Step 1: Failing tests for the pure parts (`setup.rs` `mod tests`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_env_adds_updates_and_preserves() {
        let out = upsert_env("", "TAVILY_API_KEY", "tvly-1");
        assert_eq!(out, "TAVILY_API_KEY=tvly-1\n");
        let out = upsert_env(&out, "OTHER", "x");
        let out = upsert_env(&out, "TAVILY_API_KEY", "tvly-2"); // replace in place
        assert_eq!(out.matches("TAVILY_API_KEY").count(), 1);
        assert!(out.contains("TAVILY_API_KEY=tvly-2"));
        assert!(out.contains("OTHER=x"));
        // Comments and unknown lines survive.
        let cur = "# my keys\nFOO=bar\n";
        let out = upsert_env(cur, "BAZ", "qux");
        assert!(out.starts_with("# my keys\nFOO=bar\n"));
        assert!(out.contains("BAZ=qux"));
    }

    #[test]
    fn config_from_template_substitutes_vault_path() {
        let toml = generated_config("/Users/matt/Vault");
        let cfg: construct_config::Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.vault.path, "/Users/matt/Vault");
    }

    #[test]
    fn key_specs_collects_configured_env_names() {
        let cfg: construct_config::Config =
            toml::from_str(crate::commands::SAMPLE_CONFIG).unwrap();
        let keys = key_specs(Some(&cfg));
        assert!(keys.iter().any(|k| k.env == "TAVILY_API_KEY"));
        // No config yet → still suggests the known key set.
        assert!(key_specs(None).iter().any(|k| k.env == "TAVILY_API_KEY"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p construct-cli setup`
Expected: FAIL — module missing

- [ ] **Step 3: Implement `setup.rs`**

```rust
//! Interactive first-run setup: vault path, starter config, API keys → .env.
//! Design rules: never rewrite an existing construct.toml; never echo a key;
//! .env is always chmod 600. `--non-interactive` makes this scriptable.
use anyhow::Context;
use std::path::{Path, PathBuf};

/// One promptable API key.
pub struct KeySpec {
    pub env: &'static str,
    pub label: &'static str,
}

/// Keys worth prompting for: anything the loaded config references via
/// api_key_env, plus the well-known set for features the user may enable
/// later. (When an Anthropic/hosted provider lands, add it here.)
pub fn key_specs(cfg: Option<&construct_config::Config>) -> Vec<KeySpec> {
    let mut keys = vec![KeySpec { env: "TAVILY_API_KEY", label: "Tavily (web search)" }];
    if let Some(cfg) = cfg {
        if let Some(ws) = &cfg.tools.web_search {
            if !keys.iter().any(|k| k.env == ws.api_key_env) {
                // Config names a custom env var: prompt for that instead.
                keys.insert(
                    0,
                    KeySpec { env: Box::leak(ws.api_key_env.clone().into_boxed_str()), label: "web search" },
                );
            }
        }
    }
    keys
}

/// Pure: add or replace KEY=VALUE in .env content, preserving everything else.
pub fn upsert_env(existing: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let mut found = false;
    let mut lines: Vec<String> = existing
        .lines()
        .map(|l| {
            if l.starts_with(&prefix) {
                found = true;
                format!("{key}={value}")
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(format!("{key}={value}"));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Starter config with the vault path substituted into the template.
pub fn generated_config(vault_path: &str) -> String {
    crate::commands::SAMPLE_CONFIG.replace("path = \"~/ObsidianVault\"", &format!("path = \"{vault_path}\""))
}

/// Write .env with owner-only permissions. Refuses to leave it looser.
pub fn write_env_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 600 {}", path.display()))?;
    Ok(())
}

/// Looks like an Obsidian vault (or at least an existing directory we can use).
fn validate_vault_path(input: &str) -> Result<(), String> {
    let p = PathBuf::from(shellexpand(input));
    if !p.is_dir() {
        return Err(format!("{} is not a directory", p.display()));
    }
    if !p.join(".obsidian").is_dir() {
        // Warn-level: usable, but worth flagging in the prompt loop.
        return Err(format!(
            "{} exists but has no .obsidian/ — enter it again to use anyway",
            p.display()
        ));
    }
    Ok(())
}

fn shellexpand(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    p.to_string()
}

pub struct SetupArgs {
    pub non_interactive: bool,
    pub vault: Option<String>,
    /// KEY=VALUE pairs.
    pub keys: Vec<String>,
}

/// Entry point. `config_path` is the resolved construct.toml path; `home` its dir.
pub async fn run_setup(config_path: &Path, home: &Path, args: SetupArgs) -> anyhow::Result<()> {
    println!("The Construct — setup");
    println!("  home:   {}", home.display());

    // --- construct.toml ---
    let existing_cfg = if config_path.exists() {
        let cfg = construct_config::Config::load(config_path)?;
        println!("  config: {} (existing — left untouched)", config_path.display());
        Some(cfg)
    } else {
        let vault = match (&args.vault, args.non_interactive) {
            (Some(v), _) => v.clone(),
            (None, true) => anyhow::bail!("--non-interactive requires --vault on first run"),
            (None, false) => prompt_vault_path()?,
        };
        let toml = generated_config(&shellexpand(&vault));
        std::fs::create_dir_all(home)?;
        std::fs::write(config_path, &toml)?;
        println!("  config: {} (created)", config_path.display());
        Some(construct_config::Config::load(config_path)?)
    };

    // --- API keys → .env ---
    let env_path = home.join(".env");
    let mut env_text = std::fs::read_to_string(&env_path).unwrap_or_default();
    if args.non_interactive {
        for pair in &args.keys {
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--key expects KEY=VALUE, got '{pair}'"))?;
            env_text = upsert_env(&env_text, k.trim(), v.trim());
        }
    } else {
        for spec in key_specs(existing_cfg.as_ref()) {
            let already = std::env::var(spec.env).is_ok()
                || env_text.lines().any(|l| l.starts_with(&format!("{}=", spec.env)));
            let label = if already {
                format!("{} [{}] — already set; enter to keep, or paste a new key", spec.label, spec.env)
            } else {
                format!("{} [{}] — paste key, or enter to skip", spec.label, spec.env)
            };
            let value: String = dialoguer::Password::new()
                .with_prompt(label)
                .allow_empty_password(true)
                .interact()?;
            if !value.trim().is_empty() {
                env_text = upsert_env(&env_text, spec.env, value.trim());
            }
        }
    }
    if !env_text.is_empty() {
        write_env_file(&env_path, &env_text)?;
        println!("  keys:   {} (mode 600)", env_path.display());
    }

    println!("\nSetup complete. Next:\n  entertheconstruct config-check\n  entertheconstruct watch");
    Ok(())
}

fn prompt_vault_path() -> anyhow::Result<String> {
    use dialoguer::Input;
    loop {
        let input: String = Input::new()
            .with_prompt("Path to your Obsidian vault")
            .interact_text()?;
        match validate_vault_path(&input) {
            Ok(()) => return Ok(input),
            Err(msg) => {
                eprintln!("  ⚠ {msg}");
                // Second consecutive identical answer = use anyway.
                let again: String = Input::new()
                    .with_prompt("Path to your Obsidian vault (repeat to confirm)")
                    .interact_text()?;
                if again == input {
                    return Ok(input);
                }
            }
        }
    }
}
```

- [ ] **Step 4: Wire the subcommand (commands.rs)**

```rust
    /// Interactive first-run setup: vault path, starter config, API keys.
    Setup {
        /// Run without prompts (requires --vault on first run).
        #[arg(long)]
        non_interactive: bool,
        /// Vault path (non-interactive first run).
        #[arg(long)]
        vault: Option<String>,
        /// KEY=VALUE pairs to store in .env (repeatable).
        #[arg(long = "key")]
        keys: Vec<String>,
    },
```

dispatch arm:

```rust
        Some(Command::Setup { non_interactive, vault, keys }) => {
            crate::setup::run_setup(
                &config,
                &base_dir,
                crate::setup::SetupArgs { non_interactive, vault, keys },
            )
            .await?;
        }
```

and `mod setup;` in `main.rs`.

- [ ] **Step 5: Non-interactive integration test (commands-level, in `setup.rs` tests)**

```rust
    #[tokio::test]
    async fn non_interactive_setup_creates_config_and_env_600() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("Vault");
        std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
        let home = tmp.path().join("home");
        let config = home.join("construct.toml");

        run_setup(
            &config,
            &home,
            SetupArgs {
                non_interactive: true,
                vault: Some(vault.to_string_lossy().to_string()),
                keys: vec!["TAVILY_API_KEY=tvly-test".into()],
            },
        )
        .await
        .unwrap();

        let cfg = construct_config::Config::load(&config).unwrap();
        assert_eq!(cfg.vault.path, vault.to_string_lossy());
        let env_path = home.join(".env");
        assert!(std::fs::read_to_string(&env_path).unwrap().contains("TAVILY_API_KEY=tvly-test"));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&env_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        // Re-run: config untouched, key replaceable.
        run_setup(
            &config,
            &home,
            SetupArgs { non_interactive: true, vault: None, keys: vec!["TAVILY_API_KEY=tvly-2".into()] },
        )
        .await
        .unwrap();
        let env = std::fs::read_to_string(&env_path).unwrap();
        assert!(env.contains("tvly-2"));
        assert_eq!(env.matches("TAVILY_API_KEY").count(), 1);
    }
```

(Add `tempfile.workspace = true` to construct-cli `[dev-dependencies]` if missing.)

- [ ] **Step 6: Hand off from `setup-home.sh`** — at the end of the script, after the prompt deploy, replace any "next steps" echo with:

```bash
if command -v entertheconstruct >/dev/null 2>&1; then
  exec entertheconstruct setup
else
  echo "Now run: entertheconstruct setup"
fi
```

- [ ] **Step 7: Run, then commit**

Run: `cargo test -p construct-cli`
Expected: PASS

```bash
git add Cargo.toml Cargo.lock crates/construct-cli scripts/setup-home.sh
git commit -m "feat(cli): interactive setup wizard — starter config + chmod-600 .env keys"
```

---

## Phase 5 — TUI dashboard

### Task 13: Engine event stream

**Files:**
- Create: `crates/construct-engine/src/events.rs`
- Modify: `crates/construct-engine/src/lib.rs` (add `pub mod events;`)
- Modify: `crates/construct-cli/src/tui/watch_loop.rs` (emit events; new `run_watch` signature)
- Modify: `crates/construct-cli/src/commands.rs` (call-site adjustment — temporary, finalized in Task 14)

- [ ] **Step 1: Implement `events.rs` (tiny, no TDD ceremony — it's a data type)**

```rust
//! Engine → UI event stream. The engine publishes; consumers (the dashboard)
//! subscribe via tokio broadcast. Lossy by design: a slow/absent UI must never
//! block or break pipelines (broadcast drops oldest on overflow).
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Inbox,
    Brief,
    Daily,
    Run,
    Error,
    Info,
}

impl EventKind {
    pub fn label(&self) -> &'static str {
        match self {
            EventKind::Inbox => "inbox",
            EventKind::Brief => "brief",
            EventKind::Daily => "daily",
            EventKind::Run => "run",
            EventKind::Error => "error",
            EventKind::Info => "info",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EngineEvent {
    pub kind: EventKind,
    pub message: String,
    /// Local wall-clock HH:MM:SS, formatted at emit time.
    pub time: String,
}

pub type EventSender = broadcast::Sender<EngineEvent>;

/// Create the channel. 256 is plenty: the dashboard drains continuously and
/// only a wall of simultaneous runs could overflow (oldest dropped, by design).
pub fn channel() -> (EventSender, broadcast::Receiver<EngineEvent>) {
    broadcast::channel(256)
}

/// Fire-and-forget emit; never errors (no subscribers is fine).
pub fn emit(tx: &EventSender, kind: EventKind, message: impl Into<String>) {
    let _ = tx.send(EngineEvent {
        kind,
        message: message.into(),
        time: chrono::Local::now().format("%H:%M:%S").to_string(),
    });
}
```

- [ ] **Step 2: Thread the sender through `run_watch`**

New signature:

```rust
pub async fn run_watch(
    cfg: Config,
    base_dir: PathBuf,
    events: construct_engine::events::EventSender,
    paused: Arc<std::sync::atomic::AtomicBool>,
) -> anyhow::Result<()> {
```

Emit at these points (using `use construct_engine::events::{emit, EventKind};`):

1. After the startup banner: `emit(&events, EventKind::Info, "watching started");`
2. In the `RouteTarget::Tag` arm, inside the spawned task around `o.handle(...)`:

```rust
                    let ev_tx = events.clone();
                    tokio::spawn(async move {
                        let _guard = lock.lock().await;
                        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        emit(&ev_tx, EventKind::Run, format!("{name} → #{t}"));
                        match o.handle(VaultEvent::NoteTagged { path, tag: t }).await {
                            Ok(()) => emit(&ev_tx, EventKind::Run, format!("{name} done")),
                            Err(e) => {
                                tracing::error!("handler error: {e}");
                                emit(&ev_tx, EventKind::Error, format!("{name}: {e}"));
                            }
                        }
                    });
```

(Apply the same pattern — clone `events` as `ev_tx`, emit start/done/error — to the `Inbox` arm with `EventKind::Inbox` and the `Daily`/brief arm with `EventKind::Brief`. Capture `name` before moving `path`.)

3. Pause gate, at the top of the event loop body right after `let event = tokio::select! { … }`:

```rust
        if paused.load(std::sync::atomic::Ordering::Relaxed) {
            emit(&events, EventKind::Info, "paused — event skipped (idle notes re-trigger automatically)");
            continue;
        }
```

- [ ] **Step 3: Patch the call site in commands.rs (temporary)**

```rust
        Some(Command::Watch) => {
            let cfg = Config::load(&config)?;
            let (events, _rx) = construct_engine::events::channel();
            let paused = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            crate::tui::watch_loop::run_watch(cfg, base_dir, events, paused).await?;
        }
```

(Add `construct-engine` imports as needed; the crate is already a dependency.)

- [ ] **Step 4: Run + commit**

Run: `cargo test --workspace`
Expected: PASS

```bash
git add crates/construct-engine crates/construct-cli
git commit -m "feat(engine): broadcast EngineEvent stream from the watch loop"
```

---

### Task 14: The dashboard

**Files:**
- Rewrite: `crates/construct-cli/src/tui/dashboard.rs` (it currently holds an early stub — read it first; keep anything the chat TUI imports, otherwise replace wholesale)
- Modify: `crates/construct-cli/src/commands.rs` (`Watch { headless }` + dispatch)
- Modify: `crates/construct-cli/src/theme.rs` only if it lacks styles used below (reuse `Theme::header()/body()/accent()`)

Behavior:
- `entertheconstruct watch` → dashboard when stdout is a TTY, plain log mode with `--headless` or when piped (launchd-safe).
- Engine runs in a spawned task; the dashboard is a pure consumer. If the dashboard errors, the engine keeps running headless.
- Keys: `q` quit (engine stops — process exit), `p` toggle pause, `o` open today's day note via `open` (macOS).

- [ ] **Step 1: Implement `dashboard.rs`**

```rust
//! Live dashboard for `entertheconstruct watch`: status header, activity feed,
//! pending-review count. Pure consumer of the EngineEvent broadcast — engine
//! pipelines never block on the UI.
use construct_engine::events::{EngineEvent, EventKind};
use construct_store::SqliteStore;
use construct_core::store::Store;
use crossterm::event::{self, Event, KeyCode};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

pub struct DashboardCtx {
    pub vault_path: String,
    pub day_note_path: std::path::PathBuf,
    pub daily_time: Option<String>,
    pub briefs_folder: Option<String>,
    pub db_url: String,
}

struct State {
    activity: VecDeque<(EventKind, String, String)>, // kind, time, message
    pending_review: usize,
    started: Instant,
    paused: bool,
}

pub async fn run_dashboard(
    ctx: DashboardCtx,
    mut rx: broadcast::Receiver<EngineEvent>,
    paused: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let store = SqliteStore::connect(&ctx.db_url).await?;
    let mut terminal = ratatui::init();
    let mut state = State {
        activity: VecDeque::with_capacity(200),
        pending_review: 0,
        started: Instant::now(),
        paused: false,
    };
    let mut last_refresh = Instant::now() - Duration::from_secs(10);

    let res: anyhow::Result<()> = loop {
        // Drain any pending engine events (non-blocking).
        loop {
            match rx.try_recv() {
                Ok(ev) => {
                    if state.activity.len() >= 200 {
                        state.activity.pop_back();
                    }
                    state.activity.push_front((ev.kind, ev.time, ev.message));
                }
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
        // Refresh pending-review count every 5s.
        if last_refresh.elapsed() >= Duration::from_secs(5) {
            if let Ok(runs) = store.list_runs(500).await {
                state.pending_review = runs.iter().filter(|r| r.status.as_str() == "review").count();
            }
            last_refresh = Instant::now();
        }

        if let Err(e) = terminal.draw(|f| draw(f, &ctx, &state)) {
            break Err(e.into());
        }

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                    KeyCode::Char('p') => {
                        state.paused = !state.paused;
                        paused.store(state.paused, Ordering::Relaxed);
                    }
                    KeyCode::Char('o') => {
                        let _ = std::process::Command::new("open")
                            .arg(&ctx.day_note_path)
                            .spawn();
                    }
                    _ => {}
                }
            }
        }
    };
    ratatui::restore();
    res
}

fn draw(f: &mut Frame, ctx: &DashboardCtx, state: &State) {
    use crate::theme::Theme;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header
            Constraint::Length(1), // status line
            Constraint::Min(3),    // activity
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    let up = state.started.elapsed().as_secs();
    let header = Paragraph::new(format!(
        " The Construct v{}  ·  vault: {}  ·  up {}h {:02}m",
        env!("CARGO_PKG_VERSION"),
        ctx.vault_path,
        up / 3600,
        (up % 3600) / 60,
    ))
    .style(Theme::header());
    f.render_widget(header, chunks[0]);

    let watching = if state.paused { "⏸ paused" } else { "● watching" };
    let daily = ctx
        .daily_time
        .as_deref()
        .map(|t| format!("  ⏱ daily {t}"))
        .unwrap_or_default();
    let briefs = ctx
        .briefs_folder
        .as_deref()
        .map(|b| format!("  ☀ briefs {b}/"))
        .unwrap_or_default();
    let status = Paragraph::new(format!(
        " {watching}{daily}{briefs}  ⚑ {} pending review",
        state.pending_review
    ))
    .style(Theme::body());
    f.render_widget(status, chunks[1]);

    let items: Vec<ListItem> = state
        .activity
        .iter()
        .map(|(kind, time, msg)| {
            let style = match kind {
                EventKind::Error => Style::default().fg(Color::Red),
                EventKind::Info => Style::default().fg(Color::DarkGray),
                _ => Theme::body(),
            };
            ListItem::new(format!("{time}  {:<6} {msg}", kind.label())).style(style)
        })
        .collect();
    let feed = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Activity ")
            .border_style(Theme::accent()),
    );
    f.render_widget(feed, chunks[2]);

    let footer =
        Paragraph::new(" q quit · p pause · o open today's note").style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, chunks[3]);
}
```

(If the existing `dashboard.rs` exports something `chat.rs`/`mod.rs` use, keep those exports; check `grep -n "dashboard" crates/construct-cli/src/tui/*.rs` first.)

- [ ] **Step 2: Wire `watch` dispatch (commands.rs)**

Subcommand:

```rust
    /// Run the vault watcher.
    Watch {
        /// Plain log output (no dashboard) — for launchd/background use.
        #[arg(long)]
        headless: bool,
    },
```

Dispatch:

```rust
        Some(Command::Watch { headless }) => {
            use std::io::IsTerminal;
            let cfg = Config::load(&config)?;
            let (events, rx) = construct_engine::events::channel();
            let paused = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let use_dashboard = !headless && std::io::stdout().is_terminal();
            if use_dashboard {
                let today = chrono::Local::now().date_naive();
                let journal_folder = cfg
                    .journal
                    .as_ref()
                    .map(|j| j.folder.clone())
                    .unwrap_or_else(|| "journal".to_string());
                let vault = crate::tui::watch_loop::expand_vault_path(&cfg.vault.path);
                let ctx = crate::tui::dashboard::DashboardCtx {
                    vault_path: vault.clone(),
                    day_note_path: std::path::PathBuf::from(&vault).join(
                        construct_engine::pipelines::daily::journal_day_path(&journal_folder, today),
                    ),
                    daily_time: cfg.schedule.as_ref().map(|s| s.daily_time.clone()),
                    briefs_folder: cfg.briefs.as_ref().map(|b| b.folder.clone()),
                    db_url,
                };
                // Engine in a background task; dashboard owns the terminal.
                // If the engine dies, surface it; if the dashboard dies, the
                // engine keeps running headless until ctrl-c.
                let engine = tokio::spawn(crate::tui::watch_loop::run_watch(
                    cfg, base_dir, events, paused.clone(),
                ));
                let ui = crate::tui::dashboard::run_dashboard(ctx, rx, paused).await;
                engine.abort();
                ui?;
            } else {
                crate::tui::watch_loop::run_watch(cfg, base_dir, events, paused).await?;
            }
        }
```

Make the tilde-expansion helper public for this: in `watch_loop.rs` rename `shellexpand_tilde` usages stay, and add

```rust
/// Public wrapper so commands.rs can resolve the display/vault path identically.
pub fn expand_vault_path(p: &str) -> String {
    shellexpand_tilde(p)
}
```

- [ ] **Step 3: Manual smoke test**

```bash
cargo run -p construct-cli -- watch              # in a terminal: dashboard renders, q quits cleanly
cargo run -p construct-cli -- watch --headless   # plain banner + log lines, ctrl-c exits
```

Expected: dashboard shows header/status/feed; touching a vault note produces an activity line; `p` flips to "⏸ paused"; `q` restores the terminal.

- [ ] **Step 4: Run tests + commit**

Run: `cargo test --workspace`
Expected: PASS

```bash
git add crates/construct-cli
git commit -m "feat(tui): live watch dashboard — status, activity feed, pause/open keys"
```

---

## Phase 6 — Release pass

### Task 15: Version, README, update script polish

**Files:**
- Modify: `crates/construct-cli/Cargo.toml` (`version = "0.4.0"`)
- Modify: `README.md` (rewrite top section for a stranger)
- Modify: `scripts/update.sh` (end with restart instruction)
- Modify: `RELEASE.md` (add 0.4.0 notes)

- [ ] **Step 1: Bump the binary version** — `version = "0.4.0"` in construct-cli's `[package]` (the dashboard header and `--version` read `CARGO_PKG_VERSION`).

- [ ] **Step 2: README install section** — the top of the README must get a stranger running in three commands, before any architecture talk:

```markdown
## Install

    tar -xzf theconstruct-aarch64-macos.tar.gz && cd theconstruct
    ./scripts/setup-home.sh        # creates ~/.theconstruct, hands off to the interactive wizard
    entertheconstruct watch        # live dashboard; --headless for background use

The wizard asks for your vault path and API keys (stored in `~/.theconstruct/.env`,
chmod 600). Re-run `entertheconstruct setup` anytime to rotate a key.

## Update

    ./scripts/update.sh            # backs up the DB, rebuilds, redeploys prompts
```

Then document the Slice 4 features in the feature list: journal tags, inbox index table, Daily Briefs (`[briefs]` config), rich recap (`prompts/daily_summary.md` is user-tunable), dashboard keys.

- [ ] **Step 3: `update.sh` final lines**

```bash
echo ""
echo "Update complete. Restart The Construct:"
echo "  entertheconstruct watch        (dashboard)"
echo "  entertheconstruct watch --headless   (background/launchd)"
```

- [ ] **Step 4: RELEASE.md** — add a `## 0.4.0` section listing the seven features in user-facing language (one line each).

- [ ] **Step 5: Commit**

```bash
git add crates/construct-cli/Cargo.toml Cargo.lock README.md RELEASE.md scripts/update.sh
git commit -m "chore(release): 0.4.0 — README for fresh installs, update restart hint"
```

---

### Task 16: Final verification

- [ ] **Step 1: Full quality gate**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all clean. Fix anything that surfaces; commit fixes as `fix:`/`style:`.

- [ ] **Step 2: Fresh-install walkthrough (manual)**

```bash
CONSTRUCT_HOME=$(mktemp -d) ./scripts/setup-home.sh
# wizard: point at a scratch vault with .obsidian/, paste a dummy key
CONSTRUCT_HOME=<same dir> cargo run -p construct-cli -- config-check
CONSTRUCT_HOME=<same dir> cargo run -p construct-cli -- watch
```

Verify: config created; `.env` is mode 600; config-check shows briefs/inbox/schedule states; dashboard renders.

- [ ] **Step 3: Briefs end-to-end (manual, against the real vault config)**

1. `entertheconstruct watch` running.
2. Create `<vault>/AI/DailyBriefs/<today>.md` with a heading + bullets.
3. Within ~1s: activity line `brief …`; the day note gains `daily-brief` block + recap refresh.
4. Re-save the file unchanged → no new agent call (hash guard; watch the activity feed).

- [ ] **Step 4: Commit any final fixes, then hand off**

Use superpowers:finishing-a-development-branch — push `slice-4-release-ready`, open a PR against `main` (after `prod-readiness` PR #3 merges) titled "Slice 4: Release-Ready — briefs, tags, setup wizard, dashboard".

---

## Self-review notes (kept for the executor)

- **Spec coverage:** §1 setup → Tasks 11–12; §2 tags → Tasks 2–3; §3 index → Task 4; §4 briefs → Tasks 5–8; §5 carryover → Task 1; §6 recap → Tasks 9–10; §7 TUI → Tasks 13–14; testing/release → Tasks 15–16.
- **Sequencing constraint:** Task 10 depends on Task 5 (`briefs_folder` config) and Task 6 (`parse_brief_date`); Task 8 depends on Tasks 6–7. Do not reorder phases 2→3.
- **Line numbers** cited from today's tree (commit f32302b); re-locate by searching the quoted code if drift occurs.
- **`include_str!` path check (Task 10):** `crates/construct-engine/src/pipelines/daily.rs` → `../../../../prompts/daily_summary.md` resolves to the repo-root `prompts/`. Create the prompt file (Step 4) BEFORE compiling Step 3's code.
- **Orchestrator field additions** (`prompt_dir`, `briefs_folder`) break every struct literal: the compiler lists them all — mechanical fix, see Task 10 Step 6.
