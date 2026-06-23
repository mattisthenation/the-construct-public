use crate::frontmatter::Note;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;

/// What the watcher reports to the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum VaultEvent {
    /// A note that carries a recognized trigger tag and is not yet claimed.
    NoteTagged { path: PathBuf, tag: String },
    /// A note whose construct_status changed to accepted/rejected.
    StatusChanged { path: PathBuf, status: String },
    /// A Daily Brief file (in the configured briefs folder, dated filename)
    /// was created or modified.
    BriefChanged {
        path: PathBuf,
        date: chrono::NaiveDate,
    },
}

/// Pure: given a note's text and the set of known trigger tags, classify it.
/// `status_key` is the frontmatter field holding the run status.
pub fn classify(
    path: &Path,
    text: &str,
    known_tags: &[String],
    status_key: &str,
) -> Option<VaultEvent> {
    let note = Note::parse(text);
    let status = note.get_str(status_key);

    // Decision events take priority: a human set accepted/rejected.
    if let Some(s) = &status {
        if s == "accepted" || s == "rejected" {
            return Some(VaultEvent::StatusChanged {
                path: path.to_path_buf(),
                status: s.clone(),
            });
        }
    }

    // Otherwise, a fresh tagged note with no active status is a trigger.
    if status.is_none() {
        for tag in note.tags() {
            if known_tags.contains(&tag) {
                return Some(VaultEvent::NoteTagged {
                    path: path.to_path_buf(),
                    tag,
                });
            }
        }
    }
    None
}

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

/// Spawn a debounced filesystem watcher. Sends classified events to `tx`.
/// Returns a join handle holding the watcher alive.
pub fn watch(
    vault: PathBuf,
    known_tags: Vec<String>,
    status_key: String,
    briefs_dir: Option<PathBuf>,
    tx: mpsc::UnboundedSender<VaultEvent>,
) -> notify_debouncer_full::Debouncer<notify::RecommendedWatcher, notify_debouncer_full::FileIdMap>
{
    use notify::{RecursiveMode, Watcher};
    use notify_debouncer_full::new_debouncer;

    let mut debouncer = new_debouncer(
        Duration::from_millis(500),
        None,
        move |res: notify_debouncer_full::DebounceEventResult| {
            let Ok(events) = res else {
                return;
            };
            for ev in events {
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
            }
        },
    )
    .expect("create debouncer");

    debouncer
        .watcher()
        .watch(&vault, RecursiveMode::Recursive)
        .expect("watch vault");
    debouncer
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn tags() -> Vec<String> {
        vec!["theconstruct/research".to_string()]
    }

    #[test]
    fn classifies_fresh_tagged_note() {
        let ev = classify(
            Path::new("/v/n.md"),
            "body #theconstruct/research",
            &tags(),
            "construct_status",
        );
        assert_eq!(
            ev,
            Some(VaultEvent::NoteTagged {
                path: "/v/n.md".into(),
                tag: "theconstruct/research".into()
            })
        );
    }

    #[test]
    fn ignores_tagged_note_already_in_progress() {
        let text = "---\nconstruct_status: review\n---\nbody #theconstruct/research";
        assert_eq!(
            classify(Path::new("/v/n.md"), text, &tags(), "construct_status"),
            None
        );
    }

    #[test]
    fn detects_accept_decision() {
        let text = "---\nconstruct_status: accepted\n---\nbody";
        assert_eq!(
            classify(Path::new("/v/n.md"), text, &tags(), "construct_status"),
            Some(VaultEvent::StatusChanged {
                path: "/v/n.md".into(),
                status: "accepted".into()
            })
        );
    }

    #[test]
    fn ignores_untagged_note() {
        assert_eq!(
            classify(
                Path::new("/v/n.md"),
                "plain body",
                &tags(),
                "construct_status"
            ),
            None
        );
    }

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
        assert_eq!(
            classify_path(Path::new("/v/Notes/x.md"), Some(briefs_dir)),
            None
        );
        // No briefs folder configured → never a brief.
        assert_eq!(
            classify_path(Path::new("/v/AI/DailyBriefs/2026-06-09.md"), None),
            None
        );
    }
}
