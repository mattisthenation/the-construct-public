//! Inbox pipeline pure helpers (URL extraction, recommendation block, _index log).

/// Extract up to `max` distinct URLs (http/https) from `text`, in document order.
/// Trailing punctuation and markdown/paren delimiters are stripped.
pub fn extract_urls(text: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let starts = rest.starts_with("http://") || rest.starts_with("https://");
        if starts {
            // Read until whitespace or a delimiter that cannot be part of a URL.
            let end = rest
                .find(|c: char| {
                    c.is_whitespace() || matches!(c, ')' | ']' | '>' | '"' | '\'' | '|')
                })
                .unwrap_or(rest.len());
            let raw = &rest[..end];
            // Strip trailing sentence punctuation.
            let url = raw.trim_end_matches(['.', ',', ';', ':', '!', '?']);
            if !url.is_empty() && !out.iter().any(|u| u == url) {
                out.push(url.to_string());
                if out.len() >= max {
                    break;
                }
            }
            i += end.max(1);
        } else {
            // advance one UTF-8 char
            let step = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            i += step;
        }
    }
    out
}

use construct_obsidian::block::{read_named, upsert_named, upsert_named_at_top};
use construct_obsidian::frontmatter::Note;

const RECOMMENDATION_BLOCK: &str = "inbox-recommendation";
const INDEX_BLOCK: &str = "inbox-log";

/// Render + insert the recommendation block at the top of the note, set status=review.
/// The note stays in Inbox; this is human-facing advice (Construct does not move it).
pub fn apply_recommendation(text: &str, destination: &str, reason: &str) -> String {
    let mut note = Note::parse(text);
    let content = format!(
        "> [!tip] Suggested location: **{destination}**\n> {reason}\n> \
         _Move it there (or clear `construct_status` to re-run)._"
    );
    note.body = upsert_named_at_top(&note.body, RECOMMENDATION_BLOCK, &content);
    note.set_str(crate::pipelines::STATUS_KEY, "review");
    note.to_string()
}

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
    let stem = entry
        .note_name
        .strip_suffix(".md")
        .unwrap_or(entry.note_name);

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

#[cfg(test)]
mod tests {
    use super::*;

    use construct_obsidian::frontmatter::Note;

    #[test]
    fn apply_recommendation_adds_block_and_review_status() {
        let text = crate::pipelines::apply_claim("My note body", "r1");
        let out = apply_recommendation(&text, "Reading/Articles", "looks like an article");
        let note = Note::parse(&out);
        assert_eq!(
            note.get_str(crate::pipelines::STATUS_KEY).as_deref(),
            Some("review")
        );
        assert!(note
            .body
            .trim_start()
            .starts_with("<!-- construct:inbox-recommendation:start -->"));
        assert!(out.contains("Reading/Articles"));
        assert!(out.contains("looks like an article"));
        assert!(out.contains("My note body"));
    }

    #[test]
    fn apply_recommendation_rerun_replaces_single_block() {
        let text = crate::pipelines::apply_claim("body", "r1");
        let once = apply_recommendation(&text, "A", "r1");
        let twice = apply_recommendation(&once, "B", "r2");
        assert_eq!(
            twice
                .matches("construct:inbox-recommendation:start")
                .count(),
            1
        );
        assert!(twice.contains("B"));
    }

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
        let a = IndexEntry {
            note_name: "idea.md",
            outcome: "recommended",
            destination: None,
            when: "2026-06-08",
        };
        let idx = update_index("", &a);
        // Recommended note stays in Inbox → bare wikilink by name.
        assert!(idx.contains("| [[idea]] | recommended | — | 2026-06-08 |"));

        let b = IndexEntry {
            note_name: "todo.md",
            outcome: "moved",
            destination: Some("Projects"),
            when: "2026-06-09",
        };
        let idx = update_index(&idx, &b);

        // Re-processing idea.md replaces its row.
        let a2 = IndexEntry {
            note_name: "idea.md",
            outcome: "moved",
            destination: Some("Archive"),
            when: "2026-06-09",
        };
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
        let e = IndexEntry {
            note_name: "new.md",
            outcome: "moved",
            destination: Some("Work"),
            when: "2026-06-09",
        };
        let idx = update_index(legacy, &e);
        // Legacy line becomes a row (outcome preserved verbatim, unknown columns dashed).
        assert!(idx.contains("| [[old]] | moved→Reading | — | — |"));
        assert!(idx.contains("| [[Work/new\\|new]] | moved | Work | 2026-06-09 |"));
        assert!(!idx.contains("- `old.md`"));
    }

    #[test]
    fn extracts_zero_urls() {
        assert!(extract_urls("just some text, no links", 5).is_empty());
    }

    #[test]
    fn extracts_single_url_stripping_trailing_punctuation() {
        let urls = extract_urls("see https://example.com/page. cool", 5);
        assert_eq!(urls, vec!["https://example.com/page".to_string()]);
    }

    #[test]
    fn extracts_markdown_link_url() {
        let urls = extract_urls("[a](https://example.com/x) and more", 5);
        assert_eq!(urls, vec!["https://example.com/x".to_string()]);
    }

    #[test]
    fn caps_at_max_five_and_dedupes() {
        let text = "https://a.com https://b.com https://c.com https://a.com \
                    https://d.com https://e.com https://f.com https://g.com";
        let urls = extract_urls(text, 5);
        assert_eq!(urls.len(), 5);
        assert_eq!(urls[0], "https://a.com");
        // a.com appears once despite repeating
        assert_eq!(urls.iter().filter(|u| *u == "https://a.com").count(), 1);
        assert!(!urls.contains(&"https://f.com".to_string()));
    }

    #[test]
    fn ignores_non_http_schemes() {
        assert!(extract_urls("ftp://x.com mailto:a@b.com", 5).is_empty());
    }
}
