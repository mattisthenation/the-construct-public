//! Daily-summary pipeline helpers: journal path math, deterministic checkbox
//! scraping, section assembly, and the "changed yesterday" note scan.
use chrono::{DateTime, Datelike, Local, NaiveDate};
use std::path::{Path, PathBuf};

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

use construct_obsidian::block::upsert_named;

/// Canonical form of a task line for duplicate detection: strip any leading
/// checkbox syntax, trim, collapse internal whitespace. Case is preserved —
/// "Email Bob" and "email bob" may be different tasks.
pub fn normalize_task(s: &str) -> String {
    let t = s.trim_start();
    let t = t
        .strip_prefix("- [ ]")
        .or_else(|| t.strip_prefix("- [x]"))
        .or_else(|| t.strip_prefix("- [X]"))
        .unwrap_or(t);
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

/// Build/update the four managed sections of a journal day note. `current` is the
/// existing day-note text ("" for a new note). Returns the full new text. Each
/// section is a named managed block so re-running updates in place.
pub fn render_day_note(
    current: &str,
    tasks: &[String],
    carryover: &[String],
    summary_body: &str,
    other_links: &[String],
) -> String {
    let tasks_body = render_checkbox_section("Today's Task List", tasks);
    let carry_body = render_checkbox_section("Carryover from yesterday", carryover);
    let summary_body = summary_body.trim_end().to_string();
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

/// Body-only excerpt for recap input: frontmatter stripped, cut at the last
/// full line within `max_chars`, ellipsis appended when truncated.
pub fn excerpt(text: &str, max_chars: usize) -> String {
    let body = construct_obsidian::frontmatter::Note::parse(text).body;
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

/// Strip a leading checkbox token from model-returned text so nothing the
/// recap writes can be scraped as an open task by tomorrow's carryover pass.
fn defuse_checkbox(s: &str) -> String {
    let t = s.trim_start();
    let defused = t
        .strip_prefix("- [ ]")
        .or_else(|| t.strip_prefix("- [x]"))
        .or_else(|| t.strip_prefix("- [X]"))
        .or_else(|| t.strip_prefix("[ ]"))
        .or_else(|| t.strip_prefix("[x]"))
        .or_else(|| t.strip_prefix("[X]"));
    match defused {
        Some(rest) => rest.trim_start().to_string(),
        None => s.to_string(),
    }
}

/// Render the daily-summary managed-block body from a validated recap.
/// Action items are plain bullets on purpose: checkbox syntax here would be
/// scraped into tomorrow's carryover and double-count against the task list.
/// All model-returned text is passed through `defuse_checkbox` so a highlight
/// or tldr line that happens to start with checkbox syntax can't be scraped.
pub fn render_summary_section(
    tldr: &str,
    highlights: &[String],
    action_items: &[String],
) -> String {
    let defused_tldr = tldr
        .trim()
        .lines()
        .map(defuse_checkbox)
        .collect::<Vec<_>>()
        .join("\n");
    let mut s = format!("## Yesterday summary\n\n{defused_tldr}\n");
    if !highlights.is_empty() {
        s.push_str("\n**Highlights**\n");
        for h in highlights {
            s.push_str(&format!("- {}\n", defuse_checkbox(h)));
        }
    }
    if !action_items.is_empty() {
        s.push_str("\n**Action items**\n");
        for a in action_items {
            s.push_str(&format!("- {}\n", defuse_checkbox(a)));
        }
    }
    s
}

use crate::guard::is_excluded;

/// All vault notes whose filesystem mtime falls on the local calendar day `date`,
/// excluding journal/managed/_index files (shared loop-guard) and `exclude_dirs`.
///
/// Note: this keys off the LAST modification time, so a note edited yesterday *and
/// again today* is attributed to today (and won't appear in yesterday's summary), and
/// near-midnight / clock-skew edits can land on an adjacent day. Accepted approximation;
/// a precise "what changed" view would use the run history instead.
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
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let mtime: DateTime<Local> = modified.into();
        if mtime.date_naive() == date {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn render_summary_section_defuses_checkbox_syntax_from_model_text() {
        let s = render_summary_section(
            "Did things.\n- [ ] sneaky tldr task",
            &["[ ] sneaky highlight".into()],
            &["- [x] sneaky action".into()],
        );
        assert!(!s.contains("- [ ]"), "open checkbox leaked: {s}");
        assert!(!s.contains("- [x]"), "checked checkbox leaked: {s}");
        // Content preserved, just defused.
        assert!(s.contains("sneaky tldr task"));
        assert!(s.contains("- sneaky highlight"));
        assert!(s.contains("- sneaky action"));
    }

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

    #[test]
    fn normalize_task_strips_checkbox_and_collapses_whitespace() {
        assert_eq!(normalize_task("- [ ]  buy   milk "), "buy milk");
        assert_eq!(normalize_task("buy milk"), "buy milk");
        assert_eq!(normalize_task("  call  Sam  "), "call Sam");
        assert_eq!(normalize_task("- [X] done thing"), "done thing");
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
            "buy  milk".to_string(), // already in today (normalized match) → excluded
            "old task".to_string(),  // genuinely open → carried
            "old task".to_string(),  // duplicate within yesterday → once
        ];
        assert_eq!(partition_carryover(&today, &yesterday), vec!["old task"]);
    }

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

    #[test]
    fn render_day_note_writes_four_sections_and_is_idempotent() {
        let tasks = vec!["buy milk".to_string(), "call Sam".to_string()];
        let carry = vec!["old task".to_string()];
        let summary_body = "## Yesterday summary\n\nYou edited two notes about the project.";
        let others = vec!["[[Project Plan]]".to_string()];

        let once = render_day_note("", &tasks, &carry, summary_body, &others);
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
        let twice = render_day_note(
            &once,
            &["buy milk".to_string()],
            &[],
            "## Yesterday summary\n\nNew prose.",
            &[],
        );
        assert_eq!(twice.matches("construct:daily-tasks:start").count(), 1);
        assert_eq!(twice.matches("construct:daily-summary:start").count(), 1);
        assert!(twice.contains("New prose."));
        assert!(!twice.contains("You edited two notes"));
    }
}
