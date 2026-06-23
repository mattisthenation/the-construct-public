use super::{RUN_KEY, STATUS_KEY};
use construct_obsidian::frontmatter::Note;

pub const MOVE_KEY: &str = "construct_proposed_move";
pub const REASON_KEY: &str = "construct_move_reason";
pub const MOVED_FROM_KEY: &str = "construct_moved_from";

/// Propose a move: record destination + reason in frontmatter, status=review.
pub fn apply_propose(text: &str, destination: &str, reason: &str) -> String {
    let mut note = Note::parse(text);
    note.set_str(MOVE_KEY, destination);
    note.set_str(REASON_KEY, reason);
    note.set_str(STATUS_KEY, "review");
    note.to_string()
}

/// Accept: stamp moved_from + status=done, drop proposal+run id. Pure (no FS).
/// The actual file move is done by the orchestrator using the returned destination.
pub fn apply_accept(text: &str, original_path: &str) -> String {
    let mut note = Note::parse(text);
    note.set_str(MOVED_FROM_KEY, original_path);
    note.set_str(STATUS_KEY, "done");
    note.remove(MOVE_KEY);
    note.remove(REASON_KEY);
    note.remove(RUN_KEY);
    note.to_string()
}

/// Reject: strip proposal, status=rejected.
pub fn apply_reject(text: &str) -> String {
    let mut note = Note::parse(text);
    note.remove(MOVE_KEY);
    note.remove(REASON_KEY);
    note.remove(RUN_KEY);
    note.set_str(STATUS_KEY, "rejected");
    note.to_string()
}

/// Read the proposed destination from a note (for the accept step).
pub fn proposed_destination(text: &str) -> Option<String> {
    Note::parse(text).get_str(MOVE_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propose_sets_review_and_fields() {
        let text = super::super::apply_claim("body", "r1");
        let out = apply_propose(&text, "Projects", "active");
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("review"));
        assert_eq!(note.get_str(MOVE_KEY).as_deref(), Some("Projects"));
        assert_eq!(proposed_destination(&out).as_deref(), Some("Projects"));
    }

    #[test]
    fn accept_records_moved_from_and_done() {
        let proposed = apply_propose(&super::super::apply_claim("body", "r1"), "Projects", "x");
        let out = apply_accept(&proposed, "/vault/n.md");
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("done"));
        assert_eq!(note.get_str(MOVED_FROM_KEY).as_deref(), Some("/vault/n.md"));
        assert!(note.get_str(MOVE_KEY).is_none());
    }

    #[test]
    fn reject_strips_proposal() {
        let proposed = apply_propose(&super::super::apply_claim("body", "r1"), "Projects", "x");
        let out = apply_reject(&proposed);
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("rejected"));
        assert!(note.get_str(MOVE_KEY).is_none());
    }
}
