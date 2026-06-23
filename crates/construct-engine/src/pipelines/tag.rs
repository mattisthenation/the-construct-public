use super::{RUN_KEY, STATUS_KEY};
use construct_obsidian::frontmatter::Note;

/// Apply tags: merge into frontmatter, set status=done.
pub fn apply_tags(text: &str, tags: &[String], done_tag: Option<&str>) -> String {
    let mut note = Note::parse(text);
    note.merge_tags(tags);
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

    #[test]
    fn applies_tags_and_done() {
        let text = super::super::apply_claim("---\ntags:\n- old\n---\nbody", "r1");
        let out = apply_tags(
            &text,
            &["new".into(), "old".into()],
            Some("theconstruct/done"),
        );
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("done"));
        // run id stamped by apply_claim must be cleared on completion.
        assert!(note.get_str(RUN_KEY).is_none());
        // Inspect the merged `tags:` sequence without depending on serde_yaml directly:
        // re-merging an existing tag must not duplicate it.
        let mut probe = Note::parse(&out);
        probe.merge_tags(&["new".into(), "old".into()]);
        let dumped = probe.to_string();
        assert!(dumped.contains("new"));
        assert!(dumped.contains("old"));
        // "old" appears exactly once in the tags block (de-duplicated).
        assert_eq!(out.matches("- old").count(), 1);
        assert_eq!(out.matches("- new").count(), 1);
    }
}
