use super::{RUN_KEY, STATUS_KEY};
use construct_core::types::ResearchResult;
use construct_obsidian::block::{remove_block, upsert_block};
use construct_obsidian::frontmatter::Note;

/// Render a ResearchResult into the markdown that goes inside the managed block.
pub fn render_result(r: &ResearchResult) -> String {
    let mut out = String::new();
    out.push_str("## Research\n\n");
    out.push_str(&r.summary);
    out.push_str("\n\n### Findings\n");
    for f in &r.findings {
        out.push_str(&format!("- {f}\n"));
    }
    out.push_str("\n### Sources\n");
    for s in &r.sources {
        out.push_str(&format!("- [{}]({})\n", s.title, s.url));
    }
    out
}

/// write_back: insert results + set status=review. Pure transform.
pub fn apply_write_back(text: &str, result: &ResearchResult) -> String {
    let mut note = Note::parse(text);
    note.body = upsert_block(&note.body, &render_result(result));
    note.set_str(STATUS_KEY, "review");
    note.to_string()
}

/// finalize on accept: set status=done, drop the run id, optionally add a tag. Pure.
pub fn apply_accept(text: &str, done_tag: Option<&str>) -> String {
    let mut note = Note::parse(text);
    note.set_str(STATUS_KEY, "done");
    note.remove(RUN_KEY);
    if let Some(tag) = done_tag {
        if !note.body.contains(&format!("#{tag}")) {
            note.body = format!("{}\n#{}\n", note.body.trim_end(), tag);
        }
    }
    note.to_string()
}

/// finalize on reject: remove the managed block, set status=rejected. Pure.
pub fn apply_reject(text: &str) -> String {
    let mut note = Note::parse(text);
    note.body = remove_block(&note.body);
    note.set_str(STATUS_KEY, "rejected");
    note.remove(RUN_KEY);
    note.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use construct_core::types::Source;

    fn result() -> ResearchResult {
        ResearchResult {
            summary: "Summary text".into(),
            findings: vec!["finding one".into()],
            sources: vec![Source {
                title: "Rust".into(),
                url: "https://rust-lang.org".into(),
            }],
        }
    }

    #[test]
    fn write_back_inserts_block_and_review_status() {
        let claimed = super::super::apply_claim("body", "run-1");
        let out = apply_write_back(&claimed, &result());
        assert!(out.contains("## Research"));
        assert!(out.contains("https://rust-lang.org"));
        assert_eq!(
            Note::parse(&out).get_str(STATUS_KEY).as_deref(),
            Some("review")
        );
    }

    #[test]
    fn accept_marks_done_and_tags() {
        let text = apply_write_back(&super::super::apply_claim("body", "r"), &result());
        let accepted = text.replace("review", "accepted"); // simulate human edit
        let out = apply_accept(&accepted, Some("theconstruct/done"));
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("done"));
        assert!(note.get_str(RUN_KEY).is_none());
        assert!(out.contains("#theconstruct/done"));
    }

    #[test]
    fn reject_removes_block() {
        let text = apply_write_back(&super::super::apply_claim("body", "r"), &result());
        let out = apply_reject(&text);
        assert!(!out.contains("## Research"));
        assert_eq!(
            Note::parse(&out).get_str(STATUS_KEY).as_deref(),
            Some("rejected")
        );
    }
}
