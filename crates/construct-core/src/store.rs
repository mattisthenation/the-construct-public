use crate::types::{RunId, RunStatus};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: RunId,
    pub rule: String,
    pub agent: String,
    pub note_path: String,
    pub status: RunStatus,
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("store backend error: {0}")]
    Backend(String),
    #[error("run not found: {0}")]
    NotFound(RunId),
}

/// Persistence abstraction (SQLite now; Postgres later).
#[async_trait]
pub trait Store: Send + Sync {
    async fn create_run(&self, run: &RunRecord) -> Result<(), StoreError>;
    async fn update_status(
        &self,
        id: &RunId,
        status: RunStatus,
        error: Option<String>,
    ) -> Result<(), StoreError>;
    async fn get_run(&self, id: &RunId) -> Result<RunRecord, StoreError>;
    async fn run_for_note(&self, note_path: &str) -> Result<Option<RunRecord>, StoreError>;
    async fn append_event(
        &self,
        id: &RunId,
        stage: &str,
        event: &str,
        payload: serde_json::Value,
    ) -> Result<(), StoreError>;
    async fn list_runs(&self, limit: i64) -> Result<Vec<RunRecord>, StoreError>;
    /// All runs currently in a given status (used for crash reconciliation on startup).
    async fn runs_with_status(&self, status: RunStatus) -> Result<Vec<RunRecord>, StoreError>;
    /// Read the last-run timestamp (RFC3339 string) for a named scheduled job, if any.
    async fn get_last_run(&self, job: &str) -> Result<Option<String>, StoreError>;
    /// Upsert the last-run timestamp (RFC3339 string) for a named scheduled job.
    async fn set_last_run(&self, job: &str, ts: &str) -> Result<(), StoreError>;
    /// Last-processed content hash for a Daily Brief file, by absolute path.
    async fn get_brief_hash(&self, path: &str) -> Result<Option<String>, StoreError>;
    /// Record the content hash after successfully processing a brief.
    async fn set_brief_hash(&self, path: &str, hash: &str) -> Result<(), StoreError>;
}
