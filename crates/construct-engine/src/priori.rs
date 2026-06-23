//! Priori — the judgment layer.
//!
//! Flow: `note → Priori (judge) → Determa (deterministic execution) OR escalate
//! to a model`. Priori is the gate that honors deterministic-first: given a note
//! and the pipeline its tag selected, it decides whether the work can be handled
//! by deterministic code (Determa) or genuinely needs a model.
//!
//! This keeps the thesis — "most of your agent calls didn't need to be model
//! calls" — as an explicit, testable decision rather than an implicit accident of
//! control flow. The deterministic execution itself lives in the `pipelines`
//! modules (remind-me, the file-this classifier) — collectively "Determa".

use crate::pipelines::file_this::classify;
use crate::pipelines::PipelineKind;
use construct_config::FileRule;

/// Priori's verdict for one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Handle with deterministic code — no model call. Carries a short reason for
    /// the activity log / audit trail.
    Deterministic(String),
    /// Escalate to a model (the irreducible reasoning step).
    Escalate(String),
}

impl Decision {
    pub fn is_deterministic(&self) -> bool {
        matches!(self, Decision::Deterministic(_))
    }
}

/// Judge how a note should be handled. `file_rules` is consulted only for the
/// file-this pipeline; other pipelines decide purely on their kind.
pub fn judge(kind: PipelineKind, body: &str, file_rules: &[FileRule]) -> Decision {
    match kind {
        // Always deterministic — the thesis-proving handler.
        PipelineKind::RemindMe => Decision::Deterministic("remind-me is rule-based".into()),
        // Deterministic IFF a keyword rule matches; otherwise escalate to classify.
        PipelineKind::FileThis => match classify(body, file_rules) {
            Some((folder, kw)) => Decision::Deterministic(format!("matched '{kw}' → {folder}")),
            None => Decision::Escalate("no file-this rule matched".into()),
        },
        // These genuinely need a model.
        _ => Decision::Escalate("pipeline requires reasoning".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Vec<FileRule> {
        vec![FileRule {
            any_of: vec!["k8s".into()],
            folder: "DevOps".into(),
        }]
    }

    #[test]
    fn remind_me_is_always_deterministic() {
        assert!(judge(PipelineKind::RemindMe, "anything", &[]).is_deterministic());
    }

    #[test]
    fn file_this_deterministic_on_match_escalates_otherwise() {
        assert!(judge(PipelineKind::FileThis, "about k8s", &rules()).is_deterministic());
        assert!(!judge(PipelineKind::FileThis, "about gardening", &rules()).is_deterministic());
    }

    #[test]
    fn research_escalates() {
        assert!(!judge(PipelineKind::Research, "x", &[]).is_deterministic());
    }
}
