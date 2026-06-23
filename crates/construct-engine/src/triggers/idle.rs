//! Idle-trigger decision core (pure). The poller (Plan 2) supplies file text +
//! mtime; this decides whether a top-level Inbox note is ready to process.
use crate::guard::is_excluded;
use crate::pipelines::STATUS_KEY;
use chrono::{DateTime, Local};
use construct_obsidian::frontmatter::Note;
use std::path::{Path, PathBuf};

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

/// Scan TOP-LEVEL files in `inbox_dir` and return notes ready for Inbox processing:
/// markdown files that are not loop-guarded and pass `should_process_inbox_note`
/// (idle long enough AND no existing `construct_status`). Not recursive.
///
/// `vault_root`/`journal_folder`/`managed_folder` feed the shared loop-guard.
/// `now` comes from the injected clock; `idle_minutes` from config.
pub fn scan_inbox(
    inbox_dir: &Path,
    vault_root: &Path,
    journal_folder: &str,
    managed_folder: Option<&str>,
    now: DateTime<Local>,
    idle_minutes: u64,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(inbox_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Top-level files only — skip directories (not recursive).
        if !path.is_file() {
            continue;
        }
        // Markdown notes only.
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if is_excluded(&path, vault_root, journal_folder, managed_folder) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let mtime: DateTime<Local> = modified.into();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if should_process_inbox_note(&text, mtime, now, idle_minutes) {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::fs;

    fn t(h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 6, 2, h, m, 0).unwrap()
    }

    #[test]
    fn idle_long_enough_with_no_status_triggers() {
        assert!(should_process_inbox_note(
            "a quick note",
            t(10, 0),
            t(10, 30),
            30
        ));
    }

    #[test]
    fn not_idle_enough_does_not_trigger() {
        assert!(!should_process_inbox_note(
            "a quick note",
            t(10, 0),
            t(10, 29),
            30
        ));
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

    fn set_mtime(path: &std::path::Path, dt: DateTime<Local>) {
        let ft = filetime::FileTime::from_unix_time(dt.timestamp(), 0);
        filetime::set_file_mtime(path, ft).unwrap();
    }

    #[test]
    fn scan_inbox_returns_only_idle_unprocessed_top_level_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let inbox = vault.join("Inbox");
        fs::create_dir_all(&inbox).unwrap();
        fs::create_dir_all(inbox.join("sub")).unwrap();

        let now = Local.with_ymd_and_hms(2026, 6, 2, 12, 0, 0).unwrap();

        // idle + no status → included
        let idle = inbox.join("idle.md");
        fs::write(&idle, "an old idea").unwrap();
        set_mtime(&idle, Local.with_ymd_and_hms(2026, 6, 2, 11, 0, 0).unwrap());

        // too recent → excluded
        let fresh = inbox.join("fresh.md");
        fs::write(&fresh, "just typed").unwrap();
        set_mtime(
            &fresh,
            Local.with_ymd_and_hms(2026, 6, 2, 11, 59, 0).unwrap(),
        );

        // has construct_status → excluded (no-reprocess)
        let processed = inbox.join("processed.md");
        fs::write(&processed, "---\nconstruct_status: review\n---\nbody").unwrap();
        set_mtime(
            &processed,
            Local.with_ymd_and_hms(2026, 6, 2, 8, 0, 0).unwrap(),
        );

        // _index is managed → excluded
        let index = inbox.join("_index.md");
        fs::write(&index, "log").unwrap();
        set_mtime(&index, Local.with_ymd_and_hms(2026, 6, 2, 8, 0, 0).unwrap());

        // file in a subfolder → excluded (top-level only)
        let nested = inbox.join("sub").join("nested.md");
        fs::write(&nested, "deep").unwrap();
        set_mtime(
            &nested,
            Local.with_ymd_and_hms(2026, 6, 2, 8, 0, 0).unwrap(),
        );

        let found = scan_inbox(&inbox, vault, "journal", None, now, 30);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["idle.md".to_string()]);
    }
}
