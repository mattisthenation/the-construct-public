use serde::{Deserialize, Serialize};

/// Stable identifier for a single pipeline run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(pub String);

impl RunId {
    pub fn new() -> Self {
        RunId(uuid::Uuid::new_v4().to_string())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The lifecycle state of a run. Serialized into note frontmatter and SQLite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Researching,
    Review,
    Accepted,
    Rejected,
    Done,
    Error,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Queued => "queued",
            RunStatus::Running => "running",
            RunStatus::Researching => "researching",
            RunStatus::Review => "review",
            RunStatus::Accepted => "accepted",
            RunStatus::Rejected => "rejected",
            RunStatus::Done => "done",
            RunStatus::Error => "error",
        }
    }
}

/// A structured research result the agent must produce. Validated by the gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResearchResult {
    pub summary: String,
    pub findings: Vec<String>,
    pub sources: Vec<Source>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub title: String,
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_status_round_trips_as_snake_case() {
        let json = serde_json::to_string(&RunStatus::Review).unwrap();
        assert_eq!(json, "\"review\"");
        let back: RunStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, RunStatus::Review);
        assert_eq!(RunStatus::Researching.as_str(), "researching");
    }

    #[test]
    fn running_status_round_trips() {
        assert_eq!(RunStatus::Running.as_str(), "running");
        let j = serde_json::to_string(&RunStatus::Running).unwrap();
        assert_eq!(
            serde_json::from_str::<RunStatus>(&j).unwrap(),
            RunStatus::Running
        );
    }

    #[test]
    fn run_id_is_unique() {
        assert_ne!(RunId::new(), RunId::new());
    }

    #[test]
    fn research_result_serializes() {
        let r = ResearchResult {
            summary: "s".into(),
            findings: vec!["f1".into()],
            sources: vec![Source {
                title: "t".into(),
                url: "https://x".into(),
            }],
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: ResearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
