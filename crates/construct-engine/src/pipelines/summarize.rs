use super::{RUN_KEY, STATUS_KEY};
use crate::actions::SummaryOut;
use construct_obsidian::block::upsert_named_at_top;
use construct_obsidian::frontmatter::Note;

/// Render the summary callout block.
pub fn render_summary(s: &SummaryOut) -> String {
    let mut out = String::from("> [!summary] TL;DR\n");
    for line in s.tldr.lines() {
        out.push_str(&format!("> {line}\n"));
    }
    if !s.action_items.is_empty() {
        out.push_str(">\n> **Action items**\n");
        for item in &s.action_items {
            out.push_str(&format!("> - [ ] {item}\n"));
        }
    }
    out
}

/// Apply summarize: insert/replace the summary block at the top, set status=done.
pub fn apply_summary(text: &str, s: &SummaryOut, done_tag: Option<&str>) -> String {
    let mut note = Note::parse(text);
    note.body = upsert_named_at_top(&note.body, "summary", &render_summary(s));
    note.set_str(STATUS_KEY, "done");
    note.remove(RUN_KEY);
    if let Some(tag) = done_tag {
        if !note.body.contains(&format!("#{tag}")) {
            note.body = format!("{}\n#{}\n", note.body.trim_end(), tag);
        }
    }
    note.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out() -> SummaryOut {
        SummaryOut {
            tldr: "This note is about X.".into(),
            action_items: vec!["Email Bob".into(), "Book room".into()],
        }
    }

    #[test]
    fn inserts_block_at_top_and_done() {
        let text = super::super::apply_claim("Original body here.", "r1");
        let applied = apply_summary(&text, &out(), Some("theconstruct/done"));
        let note = Note::parse(&applied);
        assert!(note
            .body
            .trim_start()
            .starts_with("<!-- construct:summary:start -->"));
        assert!(note.body.contains("TL;DR"));
        assert!(note.body.contains("- [ ] Email Bob"));
        assert!(note.body.contains("Original body here."));
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("done"));
        assert!(note.get_str(RUN_KEY).is_none());
        assert!(note.body.contains("#theconstruct/done"));
    }

    #[test]
    fn rerun_replaces_single_block() {
        let text = super::super::apply_claim("body", "r1");
        let once = apply_summary(&text, &out(), None);
        let twice = apply_summary(
            &once,
            &SummaryOut {
                tldr: "New".into(),
                action_items: vec![],
            },
            None,
        );
        assert_eq!(twice.matches("construct:summary:start").count(), 1);
        assert!(twice.contains("New"));
    }
}
