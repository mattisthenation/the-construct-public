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
    note.merge_tags(std::slice::from_ref(&tag));
    let literal = format!("#{tag}");
    let already_inline = note.body.split_whitespace().any(|tok| {
        tok.trim_end_matches(|c: char| !c.is_alphanumeric()) == literal || tok == literal
    });
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
