use crate::types::RunStatus;

/// The result of running one deterministic or agentic stage.
#[derive(Debug, Clone, PartialEq)]
pub enum StageOutcome {
    /// Proceed to the next stage; new status to persist.
    Continue(RunStatus),
    /// Park the run until an external status change resumes it.
    Pause(RunStatus),
    /// Terminal success.
    Done,
    /// Terminal failure with a message.
    Failed(String),
}
