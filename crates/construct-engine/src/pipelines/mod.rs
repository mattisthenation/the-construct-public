pub mod brief;
pub mod daily;
pub mod file_this;
pub mod inbox;
pub mod journal_tag;
pub mod organize;
pub mod remind;
pub mod research;
pub mod summarize;
pub mod tag;

/// Which built-in pipeline a rule selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineKind {
    /// Deterministic: parse "remind me to X" and record it. NEVER calls a model.
    RemindMe,
    /// Deterministic-first: keyword-route to a folder; escalate to model on miss.
    FileThis,
    Research,
    Summarize,
    Tag,
    Organize,
    Inbox,
    DailySummary,
}

impl PipelineKind {
    pub fn from_name(name: &str) -> Option<PipelineKind> {
        match name {
            // The three spec handler names are the canonical user-facing ones;
            // older internal names are kept as aliases so existing configs work.
            "remind-me" | "remind_me" => Some(PipelineKind::RemindMe),
            "research-this" | "research" => Some(PipelineKind::Research),
            "file-this" => Some(PipelineKind::FileThis),
            "organize" => Some(PipelineKind::Organize),
            "summarize" => Some(PipelineKind::Summarize),
            "tag" => Some(PipelineKind::Tag),
            "inbox" => Some(PipelineKind::Inbox),
            "daily_summary" => Some(PipelineKind::DailySummary),
            _ => None,
        }
    }
    /// Auto-apply pipelines finish without a human review step.
    pub fn is_auto_apply(&self) -> bool {
        matches!(
            self,
            PipelineKind::Summarize | PipelineKind::Tag | PipelineKind::RemindMe
        )
    }
    /// True for pipelines that run entirely deterministically — no LLM call.
    /// This is the thesis made checkable.
    pub fn is_deterministic(&self) -> bool {
        matches!(self, PipelineKind::RemindMe)
    }
}

pub const STATUS_KEY: &str = "construct_status";
pub const RUN_KEY: &str = "construct_run_id";

/// claim: stamp status=queued + run id onto the note text. Pure transform.
/// Shared by all pipelines.
pub fn apply_claim(text: &str, run_id: &str) -> String {
    use construct_obsidian::frontmatter::Note;
    let mut note = Note::parse(text);
    note.set_str(STATUS_KEY, "queued");
    note.set_str(RUN_KEY, run_id);
    note.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_kind_parses() {
        assert_eq!(PipelineKind::from_name("tag"), Some(PipelineKind::Tag));
        assert_eq!(PipelineKind::from_name("nope"), None);
        assert!(PipelineKind::Summarize.is_auto_apply());
        assert!(!PipelineKind::Organize.is_auto_apply());
    }

    #[test]
    fn new_pipeline_kinds_parse() {
        assert_eq!(PipelineKind::from_name("inbox"), Some(PipelineKind::Inbox));
        assert_eq!(
            PipelineKind::from_name("daily_summary"),
            Some(PipelineKind::DailySummary)
        );
        assert!(!PipelineKind::Inbox.is_auto_apply());
        assert!(!PipelineKind::DailySummary.is_auto_apply());
    }

    #[test]
    fn claim_sets_status_and_run() {
        use construct_obsidian::frontmatter::Note;
        let out = apply_claim("body #theconstruct/research", "run-1");
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("queued"));
        assert_eq!(note.get_str(RUN_KEY).as_deref(), Some("run-1"));
    }
}
