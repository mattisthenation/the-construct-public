use async_trait::async_trait;
use construct_core::store::{RunRecord, Store, StoreError};
use construct_core::types::{RunId, RunStatus};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

pub struct SqliteStore {
    pool: SqlitePool,
}

fn map_status(s: &str) -> RunStatus {
    match s {
        "queued" => RunStatus::Queued,
        "running" => RunStatus::Running,
        "researching" => RunStatus::Researching,
        "review" => RunStatus::Review,
        "accepted" => RunStatus::Accepted,
        "rejected" => RunStatus::Rejected,
        "done" => RunStatus::Done,
        _ => RunStatus::Error,
    }
}

impl SqliteStore {
    /// `url` example: "sqlite://construct.db" or "sqlite::memory:".
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let opts = SqliteConnectOptions::from_str(url)
            .map_err(|e| StoreError::Backend(e.to_string()))?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .connect_with(opts)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        // The DB holds an index of note paths + error text — restrict it to the owner.
        #[cfg(unix)]
        if let Some(path) = url
            .strip_prefix("sqlite://")
            .filter(|p| !p.starts_with(':') && !p.is_empty())
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(SqliteStore { pool })
    }
}

#[async_trait]
impl Store for SqliteStore {
    async fn create_run(&self, run: &RunRecord) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO runs (id, rule, agent, note_path, status, error) VALUES (?,?,?,?,?,?)",
        )
        .bind(&run.id.0)
        .bind(&run.rule)
        .bind(&run.agent)
        .bind(&run.note_path)
        .bind(run.status.as_str())
        .bind(&run.error)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(())
    }

    async fn update_status(
        &self,
        id: &RunId,
        status: RunStatus,
        error: Option<String>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE runs SET status = ?, error = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(error)
        .bind(&id.0)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(())
    }

    async fn get_run(&self, id: &RunId) -> Result<RunRecord, StoreError> {
        let row =
            sqlx::query("SELECT id, rule, agent, note_path, status, error FROM runs WHERE id = ?")
                .bind(&id.0)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?
                .ok_or_else(|| StoreError::NotFound(id.clone()))?;
        Ok(RunRecord {
            id: RunId(row.get("id")),
            rule: row.get("rule"),
            agent: row.get("agent"),
            note_path: row.get("note_path"),
            status: map_status(&row.get::<String, _>("status")),
            error: row.get("error"),
        })
    }

    async fn run_for_note(&self, note_path: &str) -> Result<Option<RunRecord>, StoreError> {
        // `rowid DESC` tiebreaks runs created in the same whole second (created_at has
        // 1-second resolution), so the *latest* run is always returned — important for
        // idempotency and the decision path after a rapid reject→re-trigger.
        let row = sqlx::query("SELECT id, rule, agent, note_path, status, error FROM runs WHERE note_path = ? ORDER BY created_at DESC, rowid DESC LIMIT 1")
            .bind(note_path)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(row.map(|row| RunRecord {
            id: RunId(row.get("id")),
            rule: row.get("rule"),
            agent: row.get("agent"),
            note_path: row.get("note_path"),
            status: map_status(&row.get::<String, _>("status")),
            error: row.get("error"),
        }))
    }

    async fn append_event(
        &self,
        id: &RunId,
        stage: &str,
        event: &str,
        payload: serde_json::Value,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO run_events (run_id, stage, event, payload) VALUES (?,?,?,?)")
            .bind(&id.0)
            .bind(stage)
            .bind(event)
            .bind(payload.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(())
    }

    async fn list_runs(&self, limit: i64) -> Result<Vec<RunRecord>, StoreError> {
        let rows = sqlx::query("SELECT id, rule, agent, note_path, status, error FROM runs ORDER BY created_at DESC, rowid DESC LIMIT ?")
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| RunRecord {
                id: RunId(row.get("id")),
                rule: row.get("rule"),
                agent: row.get("agent"),
                note_path: row.get("note_path"),
                status: map_status(&row.get::<String, _>("status")),
                error: row.get("error"),
            })
            .collect())
    }

    async fn runs_with_status(&self, status: RunStatus) -> Result<Vec<RunRecord>, StoreError> {
        let rows = sqlx::query("SELECT id, rule, agent, note_path, status, error FROM runs WHERE status = ? ORDER BY created_at ASC")
            .bind(status.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| RunRecord {
                id: RunId(row.get("id")),
                rule: row.get("rule"),
                agent: row.get("agent"),
                note_path: row.get("note_path"),
                status: map_status(&row.get::<String, _>("status")),
                error: row.get("error"),
            })
            .collect())
    }

    async fn get_last_run(&self, job: &str) -> Result<Option<String>, StoreError> {
        let row = sqlx::query("SELECT last_run FROM schedule_state WHERE job = ?")
            .bind(job)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(row.map(|r| r.get::<String, _>("last_run")))
    }

    async fn set_last_run(&self, job: &str, ts: &str) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO schedule_state (job, last_run) VALUES (?, ?)
             ON CONFLICT(job) DO UPDATE SET last_run = excluded.last_run",
        )
        .bind(job)
        .bind(ts)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(())
    }

    async fn get_brief_hash(&self, path: &str) -> Result<Option<String>, StoreError> {
        let row = sqlx::query("SELECT hash FROM brief_state WHERE path = ?")
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(row.map(|r| r.get::<String, _>(0)))
    }

    async fn set_brief_hash(&self, path: &str, hash: &str) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO brief_state (path, hash, updated_at) VALUES (?, ?, datetime('now'))
             ON CONFLICT(path) DO UPDATE SET hash = excluded.hash, updated_at = datetime('now')",
        )
        .bind(path)
        .bind(hash)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> SqliteStore {
        SqliteStore::connect("sqlite::memory:").await.unwrap()
    }

    fn rec() -> RunRecord {
        RunRecord {
            id: RunId::new(),
            rule: "research".into(),
            agent: "Scout".into(),
            note_path: "/v/note.md".into(),
            status: RunStatus::Queued,
            error: None,
        }
    }

    #[tokio::test]
    async fn create_and_get() {
        let s = store().await;
        let r = rec();
        s.create_run(&r).await.unwrap();
        let got = s.get_run(&r.id).await.unwrap();
        assert_eq!(got.status, RunStatus::Queued);
        assert_eq!(got.agent, "Scout");
    }

    #[tokio::test]
    async fn update_status_persists() {
        let s = store().await;
        let r = rec();
        s.create_run(&r).await.unwrap();
        s.update_status(&r.id, RunStatus::Review, None)
            .await
            .unwrap();
        assert_eq!(s.get_run(&r.id).await.unwrap().status, RunStatus::Review);
    }

    #[tokio::test]
    async fn run_for_note_returns_latest() {
        let s = store().await;
        let r = rec();
        s.create_run(&r).await.unwrap();
        let found = s.run_for_note("/v/note.md").await.unwrap().unwrap();
        assert_eq!(found.id, r.id);
        assert!(s.run_for_note("/v/missing.md").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn events_and_list() {
        let s = store().await;
        let r = rec();
        s.create_run(&r).await.unwrap();
        s.append_event(&r.id, "claim", "started", serde_json::json!({"x":1}))
            .await
            .unwrap();
        assert_eq!(s.list_runs(10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn schedule_state_roundtrips() {
        let store = SqliteStore::connect("sqlite::memory:").await.unwrap();
        // Absent job → None.
        assert_eq!(store.get_last_run("daily_summary").await.unwrap(), None);
        // Set then read back.
        store
            .set_last_run("daily_summary", "2026-06-01T01:00:00+00:00")
            .await
            .unwrap();
        assert_eq!(
            store
                .get_last_run("daily_summary")
                .await
                .unwrap()
                .as_deref(),
            Some("2026-06-01T01:00:00+00:00")
        );
        // Upsert overwrites.
        store
            .set_last_run("daily_summary", "2026-06-02T01:00:00+00:00")
            .await
            .unwrap();
        assert_eq!(
            store
                .get_last_run("daily_summary")
                .await
                .unwrap()
                .as_deref(),
            Some("2026-06-02T01:00:00+00:00")
        );
    }

    #[tokio::test]
    async fn brief_state_roundtrips_and_overwrites() {
        let store = SqliteStore::connect("sqlite::memory:").await.unwrap();
        assert_eq!(store.get_brief_hash("/v/b.md").await.unwrap(), None);
        store.set_brief_hash("/v/b.md", "abc").await.unwrap();
        assert_eq!(
            store.get_brief_hash("/v/b.md").await.unwrap(),
            Some("abc".into())
        );
        store.set_brief_hash("/v/b.md", "def").await.unwrap();
        assert_eq!(
            store.get_brief_hash("/v/b.md").await.unwrap(),
            Some("def".into())
        );
    }

    #[tokio::test]
    async fn runs_with_status_filters() {
        let s = store().await;
        let r = rec(); // status Queued
        s.create_run(&r).await.unwrap();
        assert_eq!(
            s.runs_with_status(RunStatus::Queued).await.unwrap().len(),
            1
        );
        assert_eq!(s.runs_with_status(RunStatus::Done).await.unwrap().len(), 0);
        s.update_status(&r.id, RunStatus::Researching, None)
            .await
            .unwrap();
        assert_eq!(
            s.runs_with_status(RunStatus::Researching)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            s.runs_with_status(RunStatus::Queued).await.unwrap().len(),
            0
        );
    }
}
