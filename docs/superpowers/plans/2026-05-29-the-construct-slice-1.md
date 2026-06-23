# The Construct — Slice 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a runnable, local-first agent runtime that watches an Obsidian vault, runs a deterministic pipeline wrapping one agentic web-research loop on a tagged note, writes a reviewable draft back, and finalizes on a frontmatter accept/reject — all driven from config and surfaced in a styled `entertheconstruct` TUI.

**Architecture:** A config-driven pipeline engine. A deterministic outer shell (watcher → rule match → ordered stages, persisted as a state machine in SQLite) wraps a single agentic stage. The agent returns structured data only; deterministic stages own every file mutation. Model access and tools sit behind traits so local Ollama (now) and frontier/SearXNG (later) are swap-in.

**Tech Stack:** Rust (edition 2021), tokio (async), reqwest (HTTP), serde/serde_json/toml, sqlx (SQLite), notify (file watching), serde_yaml (frontmatter), ratatui + crossterm (TUI), clap (CLI), thiserror/anyhow (errors), tracing (logging).

---

## File Structure

Cargo workspace at repo root. Each crate has one responsibility.

```
Cargo.toml                         # workspace
crates/
  construct-core/                  # domain types + traits, zero I/O
    src/lib.rs
    src/types.rs                   # RunStatus, RunId, Event, ResearchResult, ...
    src/model.rs                   # ModelProvider trait + ChatMessage/ToolCall types
    src/tool.rs                    # Tool trait + ToolSpec/ToolResult
    src/store.rs                   # Store trait
    src/stage.rs                   # Stage trait + StageOutcome
  construct-config/                # TOML load + validate
    src/lib.rs
  construct-store/                 # SQLite impl of Store
    src/lib.rs
    migrations/0001_init.sql
  construct-model-ollama/          # ModelProvider for Ollama
    src/lib.rs
  construct-tools/                 # web_search + web_fetch tools
    src/lib.rs
    src/web_search.rs
    src/web_fetch.rs
  construct-obsidian/              # frontmatter, managed block, watcher
    src/lib.rs
    src/frontmatter.rs
    src/block.rs
    src/watcher.rs
  construct-engine/                # orchestrator, agent loop, pipeline
    src/lib.rs
    src/rules.rs
    src/agent_loop.rs
    src/gate.rs
    src/pipeline.rs
    src/orchestrator.rs
    src/testkit.rs                 # mock ModelProvider + mock Tool (cfg(test) + pub)
  construct-cli/                   # entertheconstruct binary
    src/main.rs
    src/theme.rs
    src/commands.rs
    src/tui/mod.rs
    src/tui/dashboard.rs
    src/tui/chat.rs
```

---

## Conventions for every task

- TDD: write a failing test, run it (confirm failure), implement minimally, run it (confirm pass), commit.
- Run a single crate's tests with: `cargo test -p <crate> -- --nocapture`.
- Commit messages use Conventional Commits. Co-author line:
  `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- After each task, run `cargo build` at the workspace root to confirm the whole tree still compiles.

---

## Task 1: Workspace skeleton compiles

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/construct-core/Cargo.toml`, `crates/construct-core/src/lib.rs`
- Create: `rust-toolchain.toml`, `.gitignore`

- [ ] **Step 1: Create the workspace manifest**

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
edition = "2021"
license = "MIT"
authors = ["Matt Littlehale"]

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
toml = "0.8"
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "chrono", "macros"] }
notify = "6"
notify-debouncer-full = "0.3"
ratatui = "0.28"
crossterm = "0.28"
clap = { version = "4", features = ["derive"] }
thiserror = "1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
chrono = { version = "0.4", features = ["serde"] }
async-trait = "0.1"
uuid = { version = "1", features = ["v4", "serde"] }
tempfile = "3"
```

- [ ] **Step 2: Create the core crate manifest**

`crates/construct-core/Cargo.toml`:
```toml
[package]
name = "construct-core"
version = "0.1.0"
edition.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
async-trait.workspace = true
chrono.workspace = true
uuid.workspace = true
```

- [ ] **Step 3: Minimal lib with a smoke test**

`crates/construct-core/src/lib.rs`:
```rust
//! Core domain types and traits for The Construct.

pub fn construct_name() -> &'static str {
    "The Construct"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_set() {
        assert_eq!(construct_name(), "The Construct");
    }
}
```

- [ ] **Step 4: toolchain + gitignore**

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable"
```

`.gitignore`:
```
/target
*.db
*.db-journal
.env
```

- [ ] **Step 5: Run the test**

Run: `cargo test -p construct-core`
Expected: PASS (1 test).

- [ ] **Step 6: Commit**
```bash
git add Cargo.toml rust-toolchain.toml .gitignore crates/construct-core
git commit -m "chore: scaffold cargo workspace and construct-core crate"
```

---

## Task 2: Core domain types

**Files:**
- Create: `crates/construct-core/src/types.rs`
- Modify: `crates/construct-core/src/lib.rs`

- [ ] **Step 1: Write failing tests**

`crates/construct-core/src/types.rs`:
```rust
use serde::{Deserialize, Serialize};

/// Stable identifier for a single pipeline run.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(pub String);

impl RunId {
    pub fn new() -> Self {
        RunId(uuid::Uuid::new_v4().to_string())
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
    fn run_id_is_unique() {
        assert_ne!(RunId::new(), RunId::new());
    }

    #[test]
    fn research_result_serializes() {
        let r = ResearchResult {
            summary: "s".into(),
            findings: vec!["f1".into()],
            sources: vec![Source { title: "t".into(), url: "https://x".into() }],
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: ResearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
```

Add to `crates/construct-core/src/lib.rs`:
```rust
pub mod types;
```

- [ ] **Step 2: Run tests, expect FAIL**
Run: `cargo test -p construct-core types`
Expected: compile error / FAIL (uuid not yet imported is fine — it's in deps). If it compiles, tests should pass; if not, fix imports.

- [ ] **Step 3: Make it pass** — the code above is the implementation; resolve any compile errors.

- [ ] **Step 4: Run tests, expect PASS**
Run: `cargo test -p construct-core`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add crates/construct-core
git commit -m "feat(core): add RunId, RunStatus, ResearchResult domain types"
```

---

## Task 3: Core traits (ModelProvider, Tool, Store, Stage)

**Files:**
- Create: `crates/construct-core/src/model.rs`, `tool.rs`, `store.rs`, `stage.rs`
- Modify: `crates/construct-core/src/lib.rs`

- [ ] **Step 1: Write the trait definitions + a compile-time test**

`crates/construct-core/src/model.rs`:
```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role { System, User, Assistant, Tool }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// For assistant messages requesting tools.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// For tool-result messages: which call this answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(s: impl Into<String>) -> Self { Self::simple(Role::System, s) }
    pub fn user(s: impl Into<String>) -> Self { Self::simple(Role::User, s) }
    pub fn assistant(s: impl Into<String>) -> Self { Self::simple(Role::Assistant, s) }
    fn simple(role: Role, s: impl Into<String>) -> Self {
        ChatMessage { role, content: s.into(), tool_calls: vec![], tool_call_id: None }
    }
    pub fn tool_result(id: impl Into<String>, content: impl Into<String>) -> Self {
        ChatMessage { role: Role::Tool, content: content.into(), tool_calls: vec![], tool_call_id: Some(id.into()) }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON arguments object.
    pub arguments: serde_json::Value,
}

/// What a model returns for one completion turn.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    pub message: ChatMessage,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("model transport error: {0}")]
    Transport(String),
    #[error("model returned invalid response: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<crate::tool::ToolSpec>,
}

/// Abstraction over a chat model backend (Ollama now; frontier later).
#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ModelError>;
}
```

`crates/construct-core/src/tool.rs`:
```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Advertised tool schema handed to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments object.
    pub parameters: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool execution failed: {0}")]
    Failed(String),
    #[error("invalid arguments: {0}")]
    BadArgs(String),
}

/// A callable capability the agent loop can invoke.
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    /// `args` is the JSON object the model produced.
    async fn call(&self, args: serde_json::Value) -> Result<String, ToolError>;
}
```

`crates/construct-core/src/store.rs`:
```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::types::{RunId, RunStatus};

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
    async fn update_status(&self, id: &RunId, status: RunStatus, error: Option<String>) -> Result<(), StoreError>;
    async fn get_run(&self, id: &RunId) -> Result<RunRecord, StoreError>;
    async fn run_for_note(&self, note_path: &str) -> Result<Option<RunRecord>, StoreError>;
    async fn append_event(&self, id: &RunId, stage: &str, event: &str, payload: serde_json::Value) -> Result<(), StoreError>;
    async fn list_runs(&self, limit: i64) -> Result<Vec<RunRecord>, StoreError>;
    /// All runs currently in a given status (used for crash reconciliation on startup).
    async fn runs_with_status(&self, status: RunStatus) -> Result<Vec<RunRecord>, StoreError>;
}
```

`crates/construct-core/src/stage.rs`:
```rust
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
```

Add to `crates/construct-core/src/lib.rs`:
```rust
pub mod model;
pub mod stage;
pub mod store;
pub mod tool;
pub mod types;
```

Add a test in `crates/construct-core/src/model.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builds_messages() {
        let m = ChatMessage::user("hi");
        assert_eq!(m.role, Role::User);
        assert!(m.tool_calls.is_empty());
        let t = ChatMessage::tool_result("call_1", "ok");
        assert_eq!(t.tool_call_id.as_deref(), Some("call_1"));
    }
}
```

- [ ] **Step 2: Run, expect FAIL then resolve compile errors**
Run: `cargo test -p construct-core`
Expected: compiles and passes once `async-trait` + module wiring are correct.

- [ ] **Step 3: Run, expect PASS**
Run: `cargo test -p construct-core`

- [ ] **Step 4: Commit**
```bash
git add crates/construct-core
git commit -m "feat(core): add ModelProvider, Tool, Store, Stage traits"
```

---

## Task 4: Config loading + validation

**Files:**
- Create: `crates/construct-config/Cargo.toml`, `crates/construct-config/src/lib.rs`
- Test: inline `#[cfg(test)]` in `lib.rs`

- [ ] **Step 1: Manifest**

`crates/construct-config/Cargo.toml`:
```toml
[package]
name = "construct-config"
version = "0.1.0"
edition.workspace = true

[dependencies]
serde.workspace = true
toml.workspace = true
thiserror.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 2: Write failing tests + types**

`crates/construct-config/src/lib.rs`:
```rust
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Config {
    pub construct: ConstructMeta,
    pub vault: Vault,
    #[serde(default)]
    pub agents: Vec<Agent>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub tools: Tools,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ConstructMeta { pub name: String }

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Vault {
    pub path: String,
    #[serde(default)]
    pub managed_folder: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Agent {
    pub name: String,
    pub domain: String,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub system_prompt_file: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Rule {
    pub match_tag: String,
    pub agent: String,
    pub pipeline: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Tools {
    #[serde(default)]
    pub web_search: Option<WebSearchCfg>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct WebSearchCfg {
    pub backend: String,
    pub api_key_env: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read config: {0}")]
    Io(String),
    #[error("could not parse config: {0}")]
    Parse(String),
    #[error("invalid config: {0}")]
    Validation(String),
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(e.to_string()))?;
        let cfg: Config = toml::from_str(&text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Every rule must reference a defined agent; every agent tool must be known.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for rule in &self.rules {
            if !self.agents.iter().any(|a| a.name == rule.agent) {
                return Err(ConfigError::Validation(format!(
                    "rule for tag '{}' references unknown agent '{}'", rule.match_tag, rule.agent
                )));
            }
        }
        Ok(())
    }

    pub fn agent(&self, name: &str) -> Option<&Agent> {
        self.agents.iter().find(|a| a.name == name)
    }

    pub fn rule_for_tag(&self, tag: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.match_tag == tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn sample() -> &'static str {
        r#"
[construct]
name = "The Construct"

[vault]
path = "/tmp/vault"
managed_folder = "Construct"

[[agents]]
name = "Scout"
domain = "research"
provider = "ollama"
model = "qwen2.5:14b"
base_url = "http://localhost:11434"
tools = ["web_search", "web_fetch"]

[tools.web_search]
backend = "tavily"
api_key_env = "TAVILY_API_KEY"

[[rules]]
match_tag = "theconstruct/research"
agent = "Scout"
pipeline = "research"
"#
    }

    #[test]
    fn parses_and_validates_sample() {
        let cfg: Config = toml::from_str(sample()).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.construct.name, "The Construct");
        assert_eq!(cfg.agent("Scout").unwrap().model, "qwen2.5:14b");
        assert_eq!(cfg.rule_for_tag("theconstruct/research").unwrap().agent, "Scout");
    }

    #[test]
    fn rejects_rule_with_unknown_agent() {
        let bad = sample().replace("agent = \"Scout\"\npipeline", "agent = \"Ghost\"\npipeline");
        let cfg: Config = toml::from_str(&bad).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn load_from_file() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(sample().as_bytes()).unwrap();
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.vault.path, "/tmp/vault");
    }
}
```

- [ ] **Step 3: Run, expect FAIL (crate not yet built), then PASS**
Run: `cargo test -p construct-config`
Expected: PASS (3 tests).

- [ ] **Step 4: Commit**
```bash
git add crates/construct-config
git commit -m "feat(config): TOML config types, loading, and validation"
```

---

## Task 5: SQLite Store implementation

**Files:**
- Create: `crates/construct-store/Cargo.toml`, `src/lib.rs`, `migrations/0001_init.sql`

- [ ] **Step 1: Manifest**

`crates/construct-store/Cargo.toml`:
```toml
[package]
name = "construct-store"
version = "0.1.0"
edition.workspace = true

[dependencies]
construct-core = { path = "../construct-core" }
sqlx.workspace = true
serde_json.workspace = true
async-trait.workspace = true
thiserror.workspace = true

[dev-dependencies]
tokio.workspace = true
tempfile.workspace = true
```

- [ ] **Step 2: Migration SQL**

`crates/construct-store/migrations/0001_init.sql`:
```sql
CREATE TABLE IF NOT EXISTS runs (
    id          TEXT PRIMARY KEY,
    rule        TEXT NOT NULL,
    agent       TEXT NOT NULL,
    note_path   TEXT NOT NULL,
    status      TEXT NOT NULL,
    error       TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_runs_note ON runs(note_path);

CREATE TABLE IF NOT EXISTS run_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id      TEXT NOT NULL REFERENCES runs(id),
    stage       TEXT NOT NULL,
    event       TEXT NOT NULL,
    payload     TEXT NOT NULL,
    ts          TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_events_run ON run_events(run_id);
```

- [ ] **Step 3: Write failing tests + implementation**

`crates/construct-store/src/lib.rs`:
```rust
use async_trait::async_trait;
use construct_core::store::{RunRecord, Store, StoreError};
use construct_core::types::{RunId, RunStatus};
use sqlx::sqlite::{SqlitePoolOptions, SqliteConnectOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

pub struct SqliteStore {
    pool: SqlitePool,
}

fn map_status(s: &str) -> RunStatus {
    match s {
        "queued" => RunStatus::Queued,
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
        Ok(SqliteStore { pool })
    }
}

#[async_trait]
impl Store for SqliteStore {
    async fn create_run(&self, run: &RunRecord) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO runs (id, rule, agent, note_path, status, error) VALUES (?,?,?,?,?,?)")
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

    async fn update_status(&self, id: &RunId, status: RunStatus, error: Option<String>) -> Result<(), StoreError> {
        sqlx::query("UPDATE runs SET status = ?, error = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(status.as_str())
            .bind(error)
            .bind(&id.0)
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(())
    }

    async fn get_run(&self, id: &RunId) -> Result<RunRecord, StoreError> {
        let row = sqlx::query("SELECT id, rule, agent, note_path, status, error FROM runs WHERE id = ?")
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
        let row = sqlx::query("SELECT id, rule, agent, note_path, status, error FROM runs WHERE note_path = ? ORDER BY created_at DESC LIMIT 1")
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

    async fn append_event(&self, id: &RunId, stage: &str, event: &str, payload: serde_json::Value) -> Result<(), StoreError> {
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
        let rows = sqlx::query("SELECT id, rule, agent, note_path, status, error FROM runs ORDER BY created_at DESC LIMIT ?")
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(rows.into_iter().map(|row| RunRecord {
            id: RunId(row.get("id")),
            rule: row.get("rule"),
            agent: row.get("agent"),
            note_path: row.get("note_path"),
            status: map_status(&row.get::<String, _>("status")),
            error: row.get("error"),
        }).collect())
    }

    async fn runs_with_status(&self, status: RunStatus) -> Result<Vec<RunRecord>, StoreError> {
        let rows = sqlx::query("SELECT id, rule, agent, note_path, status, error FROM runs WHERE status = ? ORDER BY created_at ASC")
            .bind(status.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(rows.into_iter().map(|row| RunRecord {
            id: RunId(row.get("id")),
            rule: row.get("rule"),
            agent: row.get("agent"),
            note_path: row.get("note_path"),
            status: map_status(&row.get::<String, _>("status")),
            error: row.get("error"),
        }).collect())
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
        s.update_status(&r.id, RunStatus::Review, None).await.unwrap();
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
        s.append_event(&r.id, "claim", "started", serde_json::json!({"x":1})).await.unwrap();
        assert_eq!(s.list_runs(10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn runs_with_status_filters() {
        let s = store().await;
        let r = rec(); // status Queued
        s.create_run(&r).await.unwrap();
        assert_eq!(s.runs_with_status(RunStatus::Queued).await.unwrap().len(), 1);
        assert_eq!(s.runs_with_status(RunStatus::Done).await.unwrap().len(), 0);
        s.update_status(&r.id, RunStatus::Researching, None).await.unwrap();
        assert_eq!(s.runs_with_status(RunStatus::Researching).await.unwrap().len(), 1);
        assert_eq!(s.runs_with_status(RunStatus::Queued).await.unwrap().len(), 0);
    }
}
```

- [ ] **Step 4: Run, expect PASS**
Run: `cargo test -p construct-store`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**
```bash
git add crates/construct-store
git commit -m "feat(store): SQLite Store impl with runs + events and migrations"
```

---

## Task 6: Ollama ModelProvider

**Files:**
- Create: `crates/construct-model-ollama/Cargo.toml`, `src/lib.rs`

- [ ] **Step 1: Manifest**

`crates/construct-model-ollama/Cargo.toml`:
```toml
[package]
name = "construct-model-ollama"
version = "0.1.0"
edition.workspace = true

[dependencies]
construct-core = { path = "../construct-core" }
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
async-trait.workspace = true
tokio.workspace = true

[dev-dependencies]
tokio.workspace = true
```

- [ ] **Step 2: Implementation (targets Ollama's OpenAI-compatible `/v1/chat/completions`)**

`crates/construct-model-ollama/src/lib.rs`:
```rust
use async_trait::async_trait;
use construct_core::model::{
    ChatMessage, ChatRequest, ChatResponse, ModelError, ModelProvider, Role, ToolCall,
};
use serde_json::{json, Value};

pub struct OllamaProvider {
    base_url: String,
    http: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        OllamaProvider { base_url: base_url.into(), http: reqwest::Client::new() }
    }

    fn role_str(r: &Role) -> &'static str {
        match r { Role::System => "system", Role::User => "user", Role::Assistant => "assistant", Role::Tool => "tool" }
    }

    /// Build the OpenAI-compatible request body. Pure function → unit-testable.
    pub fn build_body(req: &ChatRequest) -> Value {
        let messages: Vec<Value> = req.messages.iter().map(|m| {
            let mut o = json!({ "role": Self::role_str(&m.role), "content": m.content });
            if let Some(id) = &m.tool_call_id { o["tool_call_id"] = json!(id); }
            if !m.tool_calls.is_empty() {
                o["tool_calls"] = json!(m.tool_calls.iter().map(|tc| json!({
                    "id": tc.id,
                    "type": "function",
                    "function": { "name": tc.name, "arguments": tc.arguments.to_string() }
                })).collect::<Vec<_>>());
            }
            o
        }).collect();

        let mut body = json!({ "model": req.model, "messages": messages, "stream": false });
        if !req.tools.is_empty() {
            body["tools"] = json!(req.tools.iter().map(|t| json!({
                "type": "function",
                "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
            })).collect::<Vec<_>>());
        }
        body
    }

    /// Parse one choice from an OpenAI-compatible response. Pure → unit-testable.
    pub fn parse_response(v: &Value) -> Result<ChatResponse, ModelError> {
        let msg = v["choices"].get(0).and_then(|c| c.get("message"))
            .ok_or_else(|| ModelError::Invalid("no choices[0].message".into()))?;
        let content = msg["content"].as_str().unwrap_or("").to_string();
        let mut tool_calls = vec![];
        if let Some(arr) = msg["tool_calls"].as_array() {
            for tc in arr {
                let name = tc["function"]["name"].as_str().unwrap_or_default().to_string();
                let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let arguments: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                tool_calls.push(ToolCall { id: tc["id"].as_str().unwrap_or_default().to_string(), name, arguments });
            }
        }
        Ok(ChatResponse { message: ChatMessage {
            role: Role::Assistant, content, tool_calls, tool_call_id: None,
        }})
    }
}

#[async_trait]
impl ModelProvider for OllamaProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ModelError> {
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = self.http.post(url).json(&Self::build_body(&req)).send().await
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        let v: Value = resp.json().await.map_err(|e| ModelError::Transport(e.to_string()))?;
        Self::parse_response(&v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use construct_core::tool::ToolSpec;

    #[test]
    fn body_includes_tools_and_messages() {
        let req = ChatRequest {
            model: "qwen2.5:14b".into(),
            messages: vec![ChatMessage::system("s"), ChatMessage::user("q")],
            tools: vec![ToolSpec { name: "web_search".into(), description: "d".into(), parameters: json!({"type":"object"}) }],
        };
        let body = OllamaProvider::build_body(&req);
        assert_eq!(body["model"], "qwen2.5:14b");
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert_eq!(body["tools"][0]["function"]["name"], "web_search");
    }

    #[test]
    fn parses_tool_call_response() {
        let v = json!({"choices":[{"message":{"content":"","tool_calls":[
            {"id":"call_1","function":{"name":"web_search","arguments":"{\"query\":\"rust\"}"}}
        ]}}]});
        let r = OllamaProvider::parse_response(&v).unwrap();
        assert_eq!(r.message.tool_calls.len(), 1);
        assert_eq!(r.message.tool_calls[0].name, "web_search");
        assert_eq!(r.message.tool_calls[0].arguments["query"], "rust");
    }

    #[test]
    fn parses_plain_text_response() {
        let v = json!({"choices":[{"message":{"content":"hello"}}]});
        let r = OllamaProvider::parse_response(&v).unwrap();
        assert_eq!(r.message.content, "hello");
        assert!(r.message.tool_calls.is_empty());
    }
}
```

- [ ] **Step 3: Run, expect PASS** (pure-function tests, no network)
Run: `cargo test -p construct-model-ollama`
Expected: PASS (3 tests).

- [ ] **Step 4: Commit**
```bash
git add crates/construct-model-ollama
git commit -m "feat(ollama): ModelProvider with OpenAI-compatible body/response handling"
```

---

## Task 7: web_search tool (Tavily)

**Files:**
- Create: `crates/construct-tools/Cargo.toml`, `src/lib.rs`, `src/web_search.rs`, `src/web_fetch.rs`

- [ ] **Step 1: Manifest**

`crates/construct-tools/Cargo.toml`:
```toml
[package]
name = "construct-tools"
version = "0.1.0"
edition.workspace = true

[dependencies]
construct-core = { path = "../construct-core" }
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
async-trait.workspace = true

[dev-dependencies]
tokio.workspace = true
```

- [ ] **Step 2: web_search with a pure parser test**

`crates/construct-tools/src/web_search.rs`:
```rust
use async_trait::async_trait;
use construct_core::tool::{Tool, ToolError, ToolSpec};
use serde_json::{json, Value};

/// Tavily-backed web search. SearXNG can be added as a sibling impl later.
pub struct WebSearch {
    api_key: String,
    http: reqwest::Client,
    endpoint: String,
}

impl WebSearch {
    pub fn tavily(api_key: impl Into<String>) -> Self {
        WebSearch { api_key: api_key.into(), http: reqwest::Client::new(), endpoint: "https://api.tavily.com/search".into() }
    }

    /// Pure: turn a Tavily JSON response into a compact text block for the model.
    pub fn format_results(v: &Value) -> String {
        let mut out = String::new();
        if let Some(arr) = v["results"].as_array() {
            for (i, r) in arr.iter().enumerate() {
                let title = r["title"].as_str().unwrap_or("");
                let url = r["url"].as_str().unwrap_or("");
                let content = r["content"].as_str().unwrap_or("");
                out.push_str(&format!("[{}] {}\n{}\n{}\n\n", i + 1, title, url, content));
            }
        }
        if out.is_empty() { out.push_str("No results."); }
        out
    }
}

#[async_trait]
impl Tool for WebSearch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_search".into(),
            description: "Search the web for a query and return ranked results with snippets and URLs.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Search query" } },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Value) -> Result<String, ToolError> {
        let query = args["query"].as_str().ok_or_else(|| ToolError::BadArgs("missing 'query'".into()))?;
        let body = json!({ "api_key": self.api_key, "query": query, "max_results": 5 });
        let resp = self.http.post(&self.endpoint).json(&body).send().await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        let v: Value = resp.json().await.map_err(|e| ToolError::Failed(e.to_string()))?;
        Ok(Self::format_results(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_results() {
        let v = json!({"results":[{"title":"Rust","url":"https://rust-lang.org","content":"systems lang"}]});
        let out = WebSearch::format_results(&v);
        assert!(out.contains("Rust"));
        assert!(out.contains("https://rust-lang.org"));
    }

    #[test]
    fn empty_results_message() {
        assert_eq!(WebSearch::format_results(&json!({"results":[]})), "No results.");
    }

    #[test]
    fn spec_requires_query() {
        let t = WebSearch::tavily("k");
        assert_eq!(t.spec().name, "web_search");
        assert_eq!(t.spec().parameters["required"][0], "query");
    }
}
```

- [ ] **Step 3: lib wiring**

`crates/construct-tools/src/lib.rs`:
```rust
pub mod web_fetch;
pub mod web_search;

pub use web_fetch::WebFetch;
pub use web_search::WebSearch;
```

- [ ] **Step 4: Run, expect PASS** (after Task 8 adds web_fetch this compiles fully; for now stub the module)

Create a minimal `crates/construct-tools/src/web_fetch.rs` placeholder that compiles:
```rust
// Filled in Task 8.
```
Run: `cargo test -p construct-tools web_search`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**
```bash
git add crates/construct-tools
git commit -m "feat(tools): web_search tool (Tavily) with pure result formatting"
```

---

## Task 8: web_fetch tool

**Files:**
- Modify: `crates/construct-tools/src/web_fetch.rs`

- [ ] **Step 1: Implementation + pure test (HTML→text reduction)**

`crates/construct-tools/src/web_fetch.rs`:
```rust
use async_trait::async_trait;
use construct_core::tool::{Tool, ToolError, ToolSpec};
use serde_json::{json, Value};

/// Fetch a URL and return readable text (very small HTML→text reduction).
pub struct WebFetch {
    http: reqwest::Client,
    max_chars: usize,
}

impl WebFetch {
    pub fn new() -> Self {
        WebFetch { http: reqwest::Client::new(), max_chars: 8000 }
    }

    /// Pure: strip tags/scripts and collapse whitespace. Deterministic + testable.
    pub fn html_to_text(html: &str) -> String {
        let mut out = String::with_capacity(html.len());
        let mut in_tag = false;
        let mut skip_block: Option<&'static str> = None;
        let lower = html.to_ascii_lowercase();
        let mut i = 0;
        let bytes = html.as_bytes();
        while i < bytes.len() {
            if let Some(tag) = skip_block {
                let close = format!("</{}>", tag);
                if lower[i..].starts_with(&close) { skip_block = None; i += close.len(); }
                else { i += 1; }
                continue;
            }
            if lower[i..].starts_with("<script") { skip_block = Some("script"); i += 7; in_tag = true; continue; }
            if lower[i..].starts_with("<style") { skip_block = Some("style"); i += 6; in_tag = true; continue; }
            let c = bytes[i] as char;
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => out.push(c),
                _ => {}
            }
            i += 1;
        }
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

impl Default for WebFetch {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Tool for WebFetch {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "web_fetch".into(),
            description: "Fetch a URL and return its readable text content.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "url": { "type": "string", "description": "Absolute URL to fetch" } },
                "required": ["url"]
            }),
        }
    }

    async fn call(&self, args: Value) -> Result<String, ToolError> {
        let url = args["url"].as_str().ok_or_else(|| ToolError::BadArgs("missing 'url'".into()))?;
        let resp = self.http.get(url).send().await.map_err(|e| ToolError::Failed(e.to_string()))?;
        let html = resp.text().await.map_err(|e| ToolError::Failed(e.to_string()))?;
        let mut text = Self::html_to_text(&html);
        text.truncate(self.max_chars);
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_scripts() {
        let html = "<html><head><style>x{}</style></head><body><p>Hello</p><script>var a=1;</script> world</body></html>";
        let text = WebFetch::html_to_text(html);
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn spec_requires_url() {
        assert_eq!(WebFetch::new().spec().parameters["required"][0], "url");
    }
}
```

- [ ] **Step 2: Run, expect PASS**
Run: `cargo test -p construct-tools`
Expected: PASS (5 tests total).

- [ ] **Step 3: Commit**
```bash
git add crates/construct-tools
git commit -m "feat(tools): web_fetch tool with deterministic HTML-to-text"
```

---

## Task 9: Frontmatter parse + edit

**Files:**
- Create: `crates/construct-obsidian/Cargo.toml`, `src/lib.rs`, `src/frontmatter.rs`

- [ ] **Step 1: Manifest**

`crates/construct-obsidian/Cargo.toml`:
```toml
[package]
name = "construct-obsidian"
version = "0.1.0"
edition.workspace = true

[dependencies]
construct-core = { path = "../construct-core" }
serde.workspace = true
serde_yaml.workspace = true
notify.workspace = true
notify-debouncer-full.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
tempfile.workspace = true
tokio.workspace = true
```

- [ ] **Step 2: Frontmatter module with tests**

`crates/construct-obsidian/src/frontmatter.rs`:
```rust
/// A parsed Obsidian note: optional YAML frontmatter + body.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub frontmatter: serde_yaml::Mapping,
    pub body: String,
}

impl Note {
    /// Parse a markdown string into frontmatter + body.
    pub fn parse(text: &str) -> Note {
        if let Some(rest) = text.strip_prefix("---\n") {
            if let Some(end) = rest.find("\n---\n") {
                let yaml = &rest[..end];
                let body = &rest[end + 5..];
                if let Ok(serde_yaml::Value::Mapping(m)) = serde_yaml::from_str(yaml) {
                    return Note { frontmatter: m, body: body.to_string() };
                }
            }
        }
        Note { frontmatter: serde_yaml::Mapping::new(), body: text.to_string() }
    }

    /// Serialize back to a markdown string (frontmatter only emitted if non-empty).
    pub fn to_string(&self) -> String {
        if self.frontmatter.is_empty() {
            return self.body.clone();
        }
        let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(self.frontmatter.clone()))
            .unwrap_or_default();
        format!("---\n{}---\n{}", yaml, self.body)
    }

    /// Read a string value from frontmatter.
    pub fn get_str(&self, key: &str) -> Option<String> {
        self.frontmatter.get(serde_yaml::Value::from(key)).and_then(|v| v.as_str().map(String::from))
    }

    /// Set a string value in frontmatter.
    pub fn set_str(&mut self, key: &str, value: &str) {
        self.frontmatter.insert(serde_yaml::Value::from(key), serde_yaml::Value::from(value));
    }

    pub fn remove(&mut self, key: &str) {
        self.frontmatter.remove(serde_yaml::Value::from(key));
    }

    /// Collect Obsidian inline tags (#a/b) from the body.
    pub fn tags(&self) -> Vec<String> {
        let mut tags = vec![];
        for token in self.body.split_whitespace() {
            if let Some(t) = token.strip_prefix('#') {
                let clean: String = t.chars().take_while(|c| c.is_alphanumeric() || *c == '/' || *c == '_' || *c == '-').collect();
                if !clean.is_empty() { tags.push(clean); }
            }
        }
        tags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let text = "---\ntitle: Hi\n---\nbody #theconstruct/research here";
        let note = Note::parse(text);
        assert_eq!(note.get_str("title").as_deref(), Some("Hi"));
        assert!(note.body.contains("body"));
        assert_eq!(note.tags(), vec!["theconstruct/research"]);
    }

    #[test]
    fn handles_no_frontmatter() {
        let note = Note::parse("just body");
        assert!(note.frontmatter.is_empty());
        assert_eq!(note.body, "just body");
    }

    #[test]
    fn set_and_round_trip() {
        let mut note = Note::parse("body");
        note.set_str("construct_status", "queued");
        let s = note.to_string();
        let back = Note::parse(&s);
        assert_eq!(back.get_str("construct_status").as_deref(), Some("queued"));
        assert_eq!(back.body, "body");
    }

    #[test]
    fn remove_key() {
        let mut note = Note::parse("---\na: b\n---\nx");
        note.remove("a");
        assert!(note.get_str("a").is_none());
    }
}
```

- [ ] **Step 3: lib wiring (modules added incrementally)**

`crates/construct-obsidian/src/lib.rs`:
```rust
pub mod block;
pub mod frontmatter;
pub mod watcher;

pub use frontmatter::Note;
```
Create empty `src/block.rs` and `src/watcher.rs` placeholders (filled in Tasks 10–11) so the crate compiles:
```rust
// block.rs — filled in Task 10
```
```rust
// watcher.rs — filled in Task 11
```

- [ ] **Step 4: Run, expect PASS**
Run: `cargo test -p construct-obsidian frontmatter`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**
```bash
git add crates/construct-obsidian
git commit -m "feat(obsidian): frontmatter parse/edit and inline tag extraction"
```

---

## Task 10: Managed markdown block

**Files:**
- Modify: `crates/construct-obsidian/src/block.rs`

- [ ] **Step 1: Implementation + tests**

`crates/construct-obsidian/src/block.rs`:
```rust
const START: &str = "<!-- construct:research:start -->";
const END: &str = "<!-- construct:research:end -->";

/// Insert or replace the managed research block in a note body.
pub fn upsert_block(body: &str, content: &str) -> String {
    let block = format!("{START}\n{content}\n{END}");
    if let (Some(s), Some(e)) = (body.find(START), body.find(END)) {
        let mut out = String::new();
        out.push_str(&body[..s]);
        out.push_str(&block);
        out.push_str(&body[e + END.len()..]);
        out
    } else {
        let sep = if body.ends_with('\n') || body.is_empty() { "" } else { "\n" };
        format!("{body}{sep}\n{block}\n")
    }
}

/// Remove the managed block entirely (used on reject).
pub fn remove_block(body: &str) -> String {
    if let (Some(s), Some(e)) = (body.find(START), body.find(END)) {
        let mut out = String::new();
        out.push_str(body[..s].trim_end());
        out.push_str(&body[e + END.len()..]);
        out
    } else {
        body.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_when_absent() {
        let out = upsert_block("hello", "RESULT");
        assert!(out.contains(START));
        assert!(out.contains("RESULT"));
        assert!(out.contains(END));
        assert!(out.starts_with("hello"));
    }

    #[test]
    fn replaces_when_present() {
        let first = upsert_block("hello", "OLD");
        let second = upsert_block(&first, "NEW");
        assert!(second.contains("NEW"));
        assert!(!second.contains("OLD"));
        // exactly one block
        assert_eq!(second.matches(START).count(), 1);
    }

    #[test]
    fn removes_block() {
        let with = upsert_block("hello", "X");
        let without = remove_block(&with);
        assert!(!without.contains(START));
        assert!(without.contains("hello"));
    }
}
```

- [ ] **Step 2: Run, expect PASS**
Run: `cargo test -p construct-obsidian block`
Expected: PASS (3 tests).

- [ ] **Step 3: Commit**
```bash
git add crates/construct-obsidian
git commit -m "feat(obsidian): managed research block upsert/remove"
```

---

## Task 11: Vault watcher with debounce + event emission

**Files:**
- Modify: `crates/construct-obsidian/src/watcher.rs`

- [ ] **Step 1: Define the event type + a pure classifier test**

`crates/construct-obsidian/src/watcher.rs`:
```rust
use crate::frontmatter::Note;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;

/// What the watcher reports to the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum VaultEvent {
    /// A note that carries a recognized trigger tag and is not yet claimed.
    NoteTagged { path: PathBuf, tag: String },
    /// A note whose construct_status changed to accepted/rejected.
    StatusChanged { path: PathBuf, status: String },
}

/// Pure: given a note's text and the set of known trigger tags, classify it.
/// `status_key` is the frontmatter field holding the run status.
pub fn classify(path: &Path, text: &str, known_tags: &[String], status_key: &str) -> Option<VaultEvent> {
    let note = Note::parse(text);
    let status = note.get_str(status_key);

    // Decision events take priority: a human set accepted/rejected.
    if let Some(s) = &status {
        if s == "accepted" || s == "rejected" {
            return Some(VaultEvent::StatusChanged { path: path.to_path_buf(), status: s.clone() });
        }
    }

    // Otherwise, a fresh tagged note with no active status is a trigger.
    if status.is_none() {
        for tag in note.tags() {
            if known_tags.iter().any(|k| *k == tag) {
                return Some(VaultEvent::NoteTagged { path: path.to_path_buf(), tag });
            }
        }
    }
    None
}

/// Spawn a debounced filesystem watcher. Sends classified events to `tx`.
/// Returns a join handle holding the watcher alive.
pub fn watch(
    vault: PathBuf,
    known_tags: Vec<String>,
    status_key: String,
    tx: mpsc::UnboundedSender<VaultEvent>,
) -> notify_debouncer_full::Debouncer<notify::RecommendedWatcher, notify_debouncer_full::FileIdMap> {
    use notify::RecursiveMode;
    use notify_debouncer_full::new_debouncer;

    let mut debouncer = new_debouncer(Duration::from_millis(500), None, move |res: notify_debouncer_full::DebounceEventResult| {
        let Ok(events) = res else { return; };
        for ev in events {
            for path in &ev.paths {
                if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
                let Ok(text) = std::fs::read_to_string(path) else { continue; };
                if let Some(vault_event) = classify(path, &text, &known_tags, &status_key) {
                    let _ = tx.send(vault_event);
                }
            }
        }
    }).expect("create debouncer");

    debouncer.watcher().watch(&vault, RecursiveMode::Recursive).expect("watch vault");
    debouncer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags() -> Vec<String> { vec!["theconstruct/research".to_string()] }

    #[test]
    fn classifies_fresh_tagged_note() {
        let ev = classify(Path::new("/v/n.md"), "body #theconstruct/research", &tags(), "construct_status");
        assert_eq!(ev, Some(VaultEvent::NoteTagged { path: "/v/n.md".into(), tag: "theconstruct/research".into() }));
    }

    #[test]
    fn ignores_tagged_note_already_in_progress() {
        let text = "---\nconstruct_status: review\n---\nbody #theconstruct/research";
        assert_eq!(classify(Path::new("/v/n.md"), text, &tags(), "construct_status"), None);
    }

    #[test]
    fn detects_accept_decision() {
        let text = "---\nconstruct_status: accepted\n---\nbody";
        assert_eq!(
            classify(Path::new("/v/n.md"), text, &tags(), "construct_status"),
            Some(VaultEvent::StatusChanged { path: "/v/n.md".into(), status: "accepted".into() })
        );
    }

    #[test]
    fn ignores_untagged_note() {
        assert_eq!(classify(Path::new("/v/n.md"), "plain body", &tags(), "construct_status"), None);
    }
}
```

- [ ] **Step 2: Run, expect PASS** (the `classify` tests are pure; the live `watch` fn isn't unit-tested here)
Run: `cargo test -p construct-obsidian watcher`
Expected: PASS (4 tests).

- [ ] **Step 3: Commit**
```bash
git add crates/construct-obsidian
git commit -m "feat(obsidian): debounced vault watcher with pure event classifier"
```

---

## Task 12: Engine crate + rule matching

**Files:**
- Create: `crates/construct-engine/Cargo.toml`, `src/lib.rs`, `src/rules.rs`

- [ ] **Step 1: Manifest**

`crates/construct-engine/Cargo.toml`:
```toml
[package]
name = "construct-engine"
version = "0.1.0"
edition.workspace = true

[dependencies]
construct-core = { path = "../construct-core" }
construct-config = { path = "../construct-config" }
construct-obsidian = { path = "../construct-obsidian" }
serde.workspace = true
serde_json.workspace = true
async-trait.workspace = true
tokio.workspace = true
tracing.workspace = true
thiserror.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 2: rules.rs with tests**

`crates/construct-engine/src/rules.rs`:
```rust
use construct_config::{Config, Rule};

/// Find the rule whose tag matches; returns the rule and its agent name.
pub fn match_rule<'a>(cfg: &'a Config, tag: &str) -> Option<&'a Rule> {
    cfg.rule_for_tag(tag)
}

/// The set of trigger tags the watcher should recognize, derived from config.
pub fn known_tags(cfg: &Config) -> Vec<String> {
    cfg.rules.iter().map(|r| r.match_tag.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        let toml = r#"
[construct]
name = "C"
[vault]
path = "/v"
[[agents]]
name = "Scout"
domain = "research"
provider = "ollama"
model = "m"
base_url = "http://localhost:11434"
[[rules]]
match_tag = "theconstruct/research"
agent = "Scout"
pipeline = "research"
"#;
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn matches_known_tag() {
        let c = cfg();
        let r = match_rule(&c, "theconstruct/research").unwrap();
        assert_eq!(r.agent, "Scout");
    }

    #[test]
    fn no_match_for_unknown_tag() {
        assert!(match_rule(&cfg(), "other").is_none());
    }

    #[test]
    fn known_tags_lists_triggers() {
        assert_eq!(known_tags(&cfg()), vec!["theconstruct/research"]);
    }
}
```

`crates/construct-engine/src/lib.rs`:
```rust
pub mod agent_loop;
pub mod gate;
pub mod orchestrator;
pub mod pipeline;
pub mod rules;
pub mod testkit;
```
Create placeholders for the not-yet-written modules so the crate compiles:
```rust
// agent_loop.rs / gate.rs / orchestrator.rs / pipeline.rs / testkit.rs — filled in later tasks
```

- [ ] **Step 3: Run, expect PASS**
Run: `cargo test -p construct-engine rules`
Expected: PASS (3 tests).

- [ ] **Step 4: Commit**
```bash
git add crates/construct-engine
git commit -m "feat(engine): rule matching and known-tag derivation"
```

---

## Task 13: Test kit (mock ModelProvider + mock Tool)

**Files:**
- Modify: `crates/construct-engine/src/testkit.rs`

This makes the agent loop and pipeline testable with zero network/LLM.

- [ ] **Step 1: Implement mocks + a test that they satisfy the traits**

`crates/construct-engine/src/testkit.rs`:
```rust
use async_trait::async_trait;
use construct_core::model::{ChatRequest, ChatResponse, ModelError, ModelProvider};
use construct_core::tool::{Tool, ToolError, ToolSpec};
use serde_json::{json, Value};
use std::sync::Mutex;

/// A model that returns a scripted sequence of responses, one per `chat` call.
pub struct ScriptedModel {
    responses: Mutex<std::collections::VecDeque<ChatResponse>>,
}

impl ScriptedModel {
    pub fn new(responses: Vec<ChatResponse>) -> Self {
        ScriptedModel { responses: Mutex::new(responses.into_iter().collect()) }
    }
}

#[async_trait]
impl ModelProvider for ScriptedModel {
    async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ModelError> {
        self.responses.lock().unwrap().pop_front()
            .ok_or_else(|| ModelError::Invalid("scripted model exhausted".into()))
    }
}

/// A tool that returns a fixed string and records its calls.
pub struct EchoTool {
    pub name: String,
    pub output: String,
    pub calls: Mutex<Vec<Value>>,
}

impl EchoTool {
    pub fn new(name: &str, output: &str) -> Self {
        EchoTool { name: name.into(), output: output.into(), calls: Mutex::new(vec![]) }
    }
}

#[async_trait]
impl Tool for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec { name: self.name.clone(), description: "echo".into(), parameters: json!({"type":"object"}) }
    }
    async fn call(&self, args: Value) -> Result<String, ToolError> {
        self.calls.lock().unwrap().push(args);
        Ok(self.output.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use construct_core::model::ChatMessage;

    #[tokio::test]
    async fn scripted_model_pops_in_order() {
        let m = ScriptedModel::new(vec![ChatResponse { message: ChatMessage::assistant("hi") }]);
        let req = ChatRequest { model: "m".into(), messages: vec![], tools: vec![] };
        assert_eq!(m.chat(req).await.unwrap().message.content, "hi");
    }

    #[tokio::test]
    async fn echo_tool_records_calls() {
        let t = EchoTool::new("web_search", "results");
        let out = t.call(json!({"query":"x"})).await.unwrap();
        assert_eq!(out, "results");
        assert_eq!(t.calls.lock().unwrap().len(), 1);
    }
}
```

- [ ] **Step 2: Run, expect PASS**
Run: `cargo test -p construct-engine testkit`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**
```bash
git add crates/construct-engine
git commit -m "test(engine): scripted model + echo tool test kit"
```

---

## Task 14: Agentic loop

**Files:**
- Modify: `crates/construct-engine/src/agent_loop.rs`

- [ ] **Step 1: Implement the bounded tool-calling loop with tests using the test kit**

`crates/construct-engine/src/agent_loop.rs`:
```rust
use construct_core::model::{ChatMessage, ChatRequest, ModelProvider};
use construct_core::tool::Tool;
use std::collections::HashMap;
use std::sync::Arc;

pub struct LoopConfig {
    pub model: String,
    pub max_iterations: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error("model error: {0}")]
    Model(String),
    #[error("exceeded max iterations ({0})")]
    Budget(usize),
}

/// The result of an agent loop: the final answer plus the evidence the agent
/// actually gathered (all tool outputs + every URL it passed to a tool). The
/// gate uses `evidence` to reject fabricated sources.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopOutput {
    pub content: String,
    pub evidence: String,
}

/// Run the agent loop: the model may call tools repeatedly until it answers
/// with plain content or the iteration budget is exhausted. Returns the final
/// assistant text + gathered evidence. Performs NO file side effects.
pub async fn run_loop(
    provider: &dyn ModelProvider,
    tools: &HashMap<String, Arc<dyn Tool>>,
    mut messages: Vec<ChatMessage>,
    cfg: &LoopConfig,
) -> Result<LoopOutput, LoopError> {
    let specs: Vec<_> = tools.values().map(|t| t.spec()).collect();
    let mut evidence = String::new();

    for _ in 0..cfg.max_iterations {
        let req = ChatRequest { model: cfg.model.clone(), messages: messages.clone(), tools: specs.clone() };
        let resp = provider.chat(req).await.map_err(|e| LoopError::Model(e.to_string()))?;
        let msg = resp.message;

        if msg.tool_calls.is_empty() {
            return Ok(LoopOutput { content: msg.content, evidence });
        }

        // Record the assistant's tool-call turn, then answer each call.
        let calls = msg.tool_calls.clone();
        messages.push(msg);
        for call in calls {
            // Capture any URL argument so a fetched-but-not-echoed URL still counts as evidence.
            if let Some(u) = call.arguments.get("url").and_then(|v| v.as_str()) {
                evidence.push_str(u);
                evidence.push('\n');
            }
            let result = match tools.get(&call.name) {
                Some(tool) => tool.call(call.arguments.clone()).await
                    .unwrap_or_else(|e| format!("tool error: {e}")),
                None => format!("unknown tool: {}", call.name),
            };
            evidence.push_str(&result);
            evidence.push('\n');
            messages.push(ChatMessage::tool_result(call.id, result));
        }
    }
    Err(LoopError::Budget(cfg.max_iterations))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{EchoTool, ScriptedModel};
    use construct_core::model::{ChatResponse, Role, ToolCall};
    use serde_json::json;

    fn tool_call_response(name: &str, args: serde_json::Value) -> ChatResponse {
        ChatResponse { message: ChatMessage {
            role: Role::Assistant, content: String::new(),
            tool_calls: vec![ToolCall { id: "c1".into(), name: name.into(), arguments: args }],
            tool_call_id: None,
        }}
    }

    #[tokio::test]
    async fn calls_tool_then_returns_answer() {
        let model = ScriptedModel::new(vec![
            tool_call_response("web_search", json!({"query":"rust"})),
            ChatResponse { message: ChatMessage::assistant("final answer") },
        ]);
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("web_search".into(), Arc::new(EchoTool::new("web_search", "search results")));

        let out = run_loop(&model, &tools, vec![ChatMessage::user("hi")],
            &LoopConfig { model: "m".into(), max_iterations: 5 }).await.unwrap();
        assert_eq!(out.content, "final answer");
        assert!(out.evidence.contains("search results")); // tool output captured as evidence
    }

    #[tokio::test]
    async fn enforces_iteration_budget() {
        // Model always asks for a tool → never terminates → budget hit.
        let model = ScriptedModel::new(vec![
            tool_call_response("web_search", json!({"query":"a"})),
            tool_call_response("web_search", json!({"query":"b"})),
        ]);
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("web_search".into(), Arc::new(EchoTool::new("web_search", "r")));
        let err = run_loop(&model, &tools, vec![ChatMessage::user("hi")],
            &LoopConfig { model: "m".into(), max_iterations: 2 }).await.unwrap_err();
        assert!(matches!(err, LoopError::Budget(2)));
    }
}
```

- [ ] **Step 2: Run, expect PASS**
Run: `cargo test -p construct-engine agent_loop`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**
```bash
git add crates/construct-engine
git commit -m "feat(engine): bounded agentic tool-calling loop"
```

---

## Task 15: Gate (output schema validation)

**Files:**
- Modify: `crates/construct-engine/src/gate.rs`

- [ ] **Step 1: Implement + test the deterministic output validator**

`crates/construct-engine/src/gate.rs`:
```rust
use construct_core::types::ResearchResult;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GateError {
    #[error("agent output was not valid JSON: {0}")]
    NotJson(String),
    #[error("agent output failed validation: {0}")]
    Invalid(String),
    #[error("agent cited an ungrounded source not found in gathered evidence: {0}")]
    Ungrounded(String),
}

/// Extract a JSON object from the model's free text (it may wrap JSON in prose
/// or a ```json fence), validate its shape, then verify every cited source URL
/// is grounded in `evidence` (the tool outputs + fetched URLs the agent actually
/// gathered). This rejects fabricated sources from an unreliable local model.
pub fn validate(raw: &str, evidence: &str) -> Result<ResearchResult, GateError> {
    let json_slice = extract_json(raw).ok_or_else(|| GateError::NotJson("no JSON object found".into()))?;
    let result: ResearchResult = serde_json::from_str(json_slice)
        .map_err(|e| GateError::NotJson(e.to_string()))?;
    if result.summary.trim().is_empty() {
        return Err(GateError::Invalid("summary is empty".into()));
    }
    if result.findings.is_empty() {
        return Err(GateError::Invalid("findings is empty".into()));
    }
    if result.sources.is_empty() {
        return Err(GateError::Invalid("no sources cited".into()));
    }
    for s in &result.sources {
        if s.url.trim().is_empty() || !evidence.contains(s.url.trim()) {
            return Err(GateError::Ungrounded(s.url.clone()));
        }
    }
    Ok(result)
}

/// Find the first balanced top-level {...} span. Pure + testable.
fn extract_json(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let mut depth = 0usize;
    for (i, c) in s[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 { return Some(&s[start..start + i + 1]); }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_grounded_result() {
        let raw = r#"Here you go:
        ```json
        {"summary":"S","findings":["a","b"],"sources":[{"title":"t","url":"https://example.com/x"}]}
        ```"#;
        let evidence = "search results ... https://example.com/x ... more";
        let r = validate(raw, evidence).unwrap();
        assert_eq!(r.summary, "S");
        assert_eq!(r.findings.len(), 2);
    }

    #[test]
    fn rejects_missing_findings() {
        let raw = r#"{"summary":"S","findings":[],"sources":[{"title":"t","url":"u"}]}"#;
        assert_eq!(validate(raw, "u"), Err(GateError::Invalid("findings is empty".into())));
    }

    #[test]
    fn rejects_no_sources() {
        let raw = r#"{"summary":"S","findings":["a"],"sources":[]}"#;
        assert_eq!(validate(raw, "anything"), Err(GateError::Invalid("no sources cited".into())));
    }

    #[test]
    fn rejects_ungrounded_source() {
        // The cited URL never appeared in the evidence → fabricated.
        let raw = r#"{"summary":"S","findings":["a"],"sources":[{"title":"t","url":"https://made-up.example/page"}]}"#;
        let evidence = "search results about something else entirely";
        assert_eq!(
            validate(raw, evidence),
            Err(GateError::Ungrounded("https://made-up.example/page".into()))
        );
    }

    #[test]
    fn rejects_non_json() {
        assert!(matches!(validate("no json here", ""), Err(GateError::NotJson(_))));
    }
}
```

- [ ] **Step 2: Run, expect PASS**
Run: `cargo test -p construct-engine gate`
Expected: PASS (5 tests).

- [ ] **Step 3: Commit**
```bash
git add crates/construct-engine
git commit -m "feat(engine): deterministic output gate validating ResearchResult"
```

---

## Task 16: Pipeline state machine + deterministic stages

**Files:**
- Modify: `crates/construct-engine/src/pipeline.rs`

This is the heart: a function per stage, plus a driver that maps a current status
to the next stage. Stages take a small context and return a `StageOutcome`.
File writes go through `construct-obsidian`.

- [ ] **Step 1: Implement stages + a render helper, with unit tests**

`crates/construct-engine/src/pipeline.rs`:
```rust
use construct_core::types::ResearchResult;
use construct_obsidian::block::{remove_block, upsert_block};
use construct_obsidian::frontmatter::Note;

pub const STATUS_KEY: &str = "construct_status";
pub const RUN_KEY: &str = "construct_run_id";

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

/// claim: stamp status=queued + run id onto the note text. Pure transform.
pub fn apply_claim(text: &str, run_id: &str) -> String {
    let mut note = Note::parse(text);
    note.set_str(STATUS_KEY, "queued");
    note.set_str(RUN_KEY, run_id);
    note.to_string()
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
            sources: vec![Source { title: "Rust".into(), url: "https://rust-lang.org".into() }],
        }
    }

    #[test]
    fn claim_sets_status_and_run() {
        let out = apply_claim("body #theconstruct/research", "run-1");
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("queued"));
        assert_eq!(note.get_str(RUN_KEY).as_deref(), Some("run-1"));
    }

    #[test]
    fn write_back_inserts_block_and_review_status() {
        let claimed = apply_claim("body", "run-1");
        let out = apply_write_back(&claimed, &result());
        assert!(out.contains("## Research"));
        assert!(out.contains("https://rust-lang.org"));
        assert_eq!(Note::parse(&out).get_str(STATUS_KEY).as_deref(), Some("review"));
    }

    #[test]
    fn accept_marks_done_and_tags() {
        let text = apply_write_back(&apply_claim("body", "r"), &result());
        let accepted = text.replace("review", "accepted"); // simulate human edit
        let out = apply_accept(&accepted, Some("theconstruct/done"));
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("done"));
        assert!(note.get_str(RUN_KEY).is_none());
        assert!(out.contains("#theconstruct/done"));
    }

    #[test]
    fn reject_removes_block() {
        let text = apply_write_back(&apply_claim("body", "r"), &result());
        let out = apply_reject(&text);
        assert!(!out.contains("## Research"));
        assert_eq!(Note::parse(&out).get_str(STATUS_KEY).as_deref(), Some("rejected"));
    }
}
```

- [ ] **Step 2: Run, expect PASS**
Run: `cargo test -p construct-engine pipeline`
Expected: PASS (4 tests).

- [ ] **Step 3: Commit**
```bash
git add crates/construct-engine
git commit -m "feat(engine): pure pipeline stage transforms (claim/write_back/accept/reject)"
```

---

## Task 17: Orchestrator (wires events → stages → store + files)

**Files:**
- Modify: `crates/construct-engine/src/orchestrator.rs`

This is the I/O shell that ties stages to the store, model, tools, and disk. It
is tested end-to-end with the test kit + temp files (no network).

- [ ] **Step 1: Implement the orchestrator + an end-to-end test**

`crates/construct-engine/src/orchestrator.rs`:
```rust
use crate::agent_loop::{run_loop, LoopConfig};
use crate::gate;
use crate::pipeline::{apply_accept, apply_claim, apply_reject, apply_write_back};
use construct_core::model::{ChatMessage, ModelProvider};
use construct_core::store::{RunRecord, Store};
use construct_core::tool::Tool;
use construct_core::types::{RunId, RunStatus};
use construct_obsidian::frontmatter::Note;
use construct_obsidian::watcher::VaultEvent;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub struct Orchestrator {
    pub store: Arc<dyn Store>,
    pub provider: Arc<dyn ModelProvider>,
    pub tools: HashMap<String, Arc<dyn Tool>>,
    pub model: String,
    pub agent: String,
    pub rule: String,
    pub system_prompt: String,
    pub max_iterations: usize,
    pub done_tag: Option<String>,
}

impl Orchestrator {
    /// Handle one classified vault event end-to-end.
    pub async fn handle(&self, event: VaultEvent) -> anyhow::Result<()> {
        match event {
            VaultEvent::NoteTagged { path, .. } => self.handle_tagged(&path).await,
            VaultEvent::StatusChanged { path, status } => self.handle_decision(&path, &status).await,
        }
    }

    /// Crash recovery: re-trigger any run left mid-flight (queued/researching)
    /// after a restart. Runs parked in `review` are left alone (awaiting human).
    /// Call once on startup before beginning to watch.
    pub async fn reconcile(&self) -> anyhow::Result<()> {
        for status in [RunStatus::Queued, RunStatus::Researching] {
            for run in self.store.runs_with_status(status).await? {
                self.store.update_status(&run.id, RunStatus::Error, Some("reconciled after restart".into())).await?;
                self.store.append_event(&run.id, "reconcile", "restarted", serde_json::json!({})).await?;
                self.handle(VaultEvent::NoteTagged { path: run.note_path.clone().into(), tag: String::new() }).await?;
            }
        }
        Ok(())
    }

    async fn handle_tagged(&self, path: &Path) -> anyhow::Result<()> {
        let note_path = path.to_string_lossy().to_string();

        // Idempotency: skip if a non-terminal run already exists for this note.
        if let Some(existing) = self.store.run_for_note(&note_path).await? {
            if !matches!(existing.status, RunStatus::Done | RunStatus::Rejected | RunStatus::Error) {
                return Ok(());
            }
        }

        let run_id = RunId::new();
        let original = std::fs::read_to_string(path)?;

        // 1. claim
        self.store.create_run(&RunRecord {
            id: run_id.clone(), rule: self.rule.clone(), agent: self.agent.clone(),
            note_path: note_path.clone(), status: RunStatus::Queued, error: None,
        }).await?;
        std::fs::write(path, apply_claim(&original, &run_id.0))?;
        self.store.append_event(&run_id, "claim", "queued", serde_json::json!({})).await?;

        // 2. research (agent) — status researching
        self.store.update_status(&run_id, RunStatus::Researching, None).await?;
        let note = Note::parse(&original);
        let user_prompt = format!("Title/topic and note body follow. Research it on the web and return STRICT JSON matching {{summary, findings[], sources[{{title,url}}]}}.\n\n{}", note.body);
        let messages = vec![ChatMessage::system(&self.system_prompt), ChatMessage::user(user_prompt)];
        let out = match run_loop(self.provider.as_ref(), &self.tools, messages,
            &LoopConfig { model: self.model.clone(), max_iterations: self.max_iterations }).await {
            Ok(r) => r,
            Err(e) => return self.fail(&run_id, path, &e.to_string()).await,
        };

        // 3. gate (shape + source grounding against gathered evidence)
        let result = match gate::validate(&out.content, &out.evidence) {
            Ok(r) => r,
            Err(e) => return self.fail(&run_id, path, &e.to_string()).await,
        };

        // 4. write_back — status review (re-read current file to preserve edits)
        let current = std::fs::read_to_string(path)?;
        std::fs::write(path, apply_write_back(&current, &result))?;
        self.store.update_status(&run_id, RunStatus::Review, None).await?;
        self.store.append_event(&run_id, "write_back", "review", serde_json::json!({"sources": result.sources.len()})).await?;
        // 5. await_decision: simply return; the watcher resumes us on StatusChanged.
        Ok(())
    }

    async fn handle_decision(&self, path: &Path, status: &str) -> anyhow::Result<()> {
        let note_path = path.to_string_lossy().to_string();
        let Some(run) = self.store.run_for_note(&note_path).await? else { return Ok(()); };
        if run.status != RunStatus::Review { return Ok(()); }
        let current = std::fs::read_to_string(path)?;
        match status {
            "accepted" => {
                std::fs::write(path, apply_accept(&current, self.done_tag.as_deref()))?;
                self.store.update_status(&run.id, RunStatus::Done, None).await?;
                self.store.append_event(&run.id, "finalize", "done", serde_json::json!({})).await?;
            }
            "rejected" => {
                std::fs::write(path, apply_reject(&current))?;
                self.store.update_status(&run.id, RunStatus::Rejected, None).await?;
                self.store.append_event(&run.id, "finalize", "rejected", serde_json::json!({})).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn fail(&self, run_id: &RunId, path: &Path, msg: &str) -> anyhow::Result<()> {
        let current = std::fs::read_to_string(path).unwrap_or_default();
        let mut note = Note::parse(&current);
        note.set_str(crate::pipeline::STATUS_KEY, "error");
        let _ = std::fs::write(path, note.to_string());
        self.store.update_status(run_id, RunStatus::Error, Some(msg.to_string())).await?;
        self.store.append_event(run_id, "error", "failed", serde_json::json!({"message": msg})).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{EchoTool, ScriptedModel};
    use construct_core::model::{ChatResponse, Role, ToolCall};
    use construct_store::SqliteStore;
    use std::io::Write;

    async fn orch(provider: Arc<dyn ModelProvider>) -> Orchestrator {
        let store = Arc::new(SqliteStore::connect("sqlite::memory:").await.unwrap());
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        // Output includes the URL the scripted model will cite, so the grounding gate passes.
        tools.insert("web_search".into(), Arc::new(EchoTool::new("web_search", "Rust lang https://rust-lang.org systems programming")));
        Orchestrator {
            store, provider, tools,
            model: "m".into(), agent: "Scout".into(), rule: "research".into(),
            system_prompt: "You are Scout.".into(), max_iterations: 5,
            done_tag: Some("theconstruct/done".into()),
        }
    }

    fn search_then_answer() -> ScriptedModel {
        let tool_turn = ChatResponse { message: ChatMessage {
            role: Role::Assistant, content: String::new(),
            tool_calls: vec![ToolCall { id: "c1".into(), name: "web_search".into(), arguments: serde_json::json!({"query":"x"}) }],
            tool_call_id: None,
        }};
        let answer = ChatResponse { message: ChatMessage::assistant(
            r#"{"summary":"Found it","findings":["a","b"],"sources":[{"title":"Rust","url":"https://rust-lang.org"}]}"#
        )};
        ScriptedModel::new(vec![tool_turn, answer])
    }

    #[tokio::test]
    async fn full_research_then_accept_flow() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("topic.md");
        let mut f = std::fs::File::create(&note_path).unwrap();
        write!(f, "Research the Rust language #theconstruct/research").unwrap();
        drop(f);

        let o = orch(Arc::new(search_then_answer())).await;

        // tagged → research → review
        o.handle(VaultEvent::NoteTagged { path: note_path.clone(), tag: "theconstruct/research".into() }).await.unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("## Research"));
        assert!(after.contains("construct_status: review"));

        // human accepts
        let accepted = after.replace("construct_status: review", "construct_status: accepted");
        std::fs::write(&note_path, &accepted).unwrap();
        o.handle(VaultEvent::StatusChanged { path: note_path.clone(), status: "accepted".into() }).await.unwrap();

        let done = std::fs::read_to_string(&note_path).unwrap();
        assert!(done.contains("construct_status: done"));
        assert!(done.contains("#theconstruct/done"));
        let runs = o.store.list_runs(10).await.unwrap();
        assert_eq!(runs[0].status, RunStatus::Done);
    }

    #[tokio::test]
    async fn bad_output_sets_error_status() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("topic.md");
        std::fs::write(&note_path, "Topic #theconstruct/research").unwrap();

        // Model answers with non-JSON → gate fails.
        let model = ScriptedModel::new(vec![ChatResponse { message: ChatMessage::assistant("not json") }]);
        let o = orch(Arc::new(model)).await;
        o.handle(VaultEvent::NoteTagged { path: note_path.clone(), tag: "theconstruct/research".into() }).await.unwrap();

        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: error"));
    }

    #[tokio::test]
    async fn second_trigger_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("topic.md");
        std::fs::write(&note_path, "Topic #theconstruct/research").unwrap();
        let o = orch(Arc::new(search_then_answer())).await;
        o.handle(VaultEvent::NoteTagged { path: note_path.clone(), tag: "theconstruct/research".into() }).await.unwrap();
        // a second tagged event while in review must not start a new run
        o.handle(VaultEvent::NoteTagged { path: note_path.clone(), tag: "theconstruct/research".into() }).await.unwrap();
        assert_eq!(o.store.list_runs(10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reconcile_restarts_stale_research_run() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("topic.md");
        std::fs::write(&note_path, "Topic #theconstruct/research").unwrap();
        let o = orch(Arc::new(search_then_answer())).await;

        // Simulate a crash: a run left stuck in `researching`.
        let stale = RunId::new();
        o.store.create_run(&RunRecord {
            id: stale.clone(), rule: "research".into(), agent: "Scout".into(),
            note_path: note_path.to_string_lossy().to_string(),
            status: RunStatus::Researching, error: None,
        }).await.unwrap();

        o.reconcile().await.unwrap();

        // Stale run is marked error; a fresh run drove the note to review.
        assert_eq!(o.store.get_run(&stale).await.unwrap().status, RunStatus::Error);
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: review"));
    }
}
```

Add `construct-store` to the engine's `[dev-dependencies]` in `crates/construct-engine/Cargo.toml`:
```toml
[dev-dependencies]
tempfile.workspace = true
construct-store = { path = "../construct-store" }
tokio.workspace = true
```
And add `anyhow.workspace = true` to the engine's `[dependencies]`.

- [ ] **Step 2: Run, expect PASS**
Run: `cargo test -p construct-engine orchestrator`
Expected: PASS (4 tests). This is the primary end-to-end regression guard.

- [ ] **Step 3: Commit**
```bash
git add crates/construct-engine
git commit -m "feat(engine): orchestrator wiring events to stages, store, and files"
```

---

## Task 18: Theme module (earthy blues & browns)

**Files:**
- Create: `crates/construct-cli/Cargo.toml`, `src/main.rs`, `src/theme.rs`

- [ ] **Step 1: Manifest + binary name**

`crates/construct-cli/Cargo.toml`:
```toml
[package]
name = "construct-cli"
version = "0.1.0"
edition.workspace = true

[[bin]]
name = "entertheconstruct"
path = "src/main.rs"

[dependencies]
construct-core = { path = "../construct-core" }
construct-config = { path = "../construct-config" }
construct-store = { path = "../construct-store" }
construct-engine = { path = "../construct-engine" }
construct-obsidian = { path = "../construct-obsidian" }
construct-tools = { path = "../construct-tools" }
construct-model-ollama = { path = "../construct-model-ollama" }
ratatui.workspace = true
crossterm.workspace = true
clap.workspace = true
tokio.workspace = true
anyhow.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
serde_json.workspace = true
```

- [ ] **Step 2: Theme with a test on the palette**

`crates/construct-cli/src/theme.rs`:
```rust
use ratatui::style::{Color, Style};

/// Earthy blues & browns palette for The Construct.
pub struct Theme;

impl Theme {
    pub const DEEP_BLUE: Color = Color::Rgb(38, 70, 83);    // slate teal-blue
    pub const DUSK_BLUE: Color = Color::Rgb(69, 105, 124);  // muted steel blue
    pub const CLAY: Color = Color::Rgb(122, 85, 58);        // warm brown
    pub const SAND: Color = Color::Rgb(196, 164, 132);      // tan
    pub const PARCHMENT: Color = Color::Rgb(231, 217, 196); // light foreground

    pub fn header() -> Style { Style::default().fg(Self::SAND).bg(Self::DEEP_BLUE) }
    pub fn accent() -> Style { Style::default().fg(Self::CLAY) }
    pub fn body() -> Style { Style::default().fg(Self::PARCHMENT) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_is_earthy() {
        assert_eq!(Theme::DEEP_BLUE, Color::Rgb(38, 70, 83));
        assert_eq!(Theme::CLAY, Color::Rgb(122, 85, 58));
        // header uses sand on deep blue
        assert_eq!(Theme::header(), Style::default().fg(Theme::SAND).bg(Theme::DEEP_BLUE));
    }
}
```

- [ ] **Step 3: Minimal main so the crate builds**

`crates/construct-cli/src/main.rs`:
```rust
mod commands;
mod theme;
mod tui;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    commands::run().await
}
```
Create `src/commands.rs` and `src/tui/mod.rs` placeholders (filled in Tasks 19–21):
```rust
// commands.rs — filled in Task 19
pub async fn run() -> anyhow::Result<()> { Ok(()) }
```
```rust
// tui/mod.rs — filled in Task 20
```

- [ ] **Step 4: Run, expect PASS**
Run: `cargo test -p construct-cli theme`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**
```bash
git add crates/construct-cli
git commit -m "feat(cli): scaffold entertheconstruct binary + earthy theme"
```

---

## Task 19: CLI subcommands (init, config check, watch, status, runs)

**Files:**
- Modify: `crates/construct-cli/src/commands.rs`

- [ ] **Step 1: Implement command parsing + an `init` that writes a sample config, with a test**

`crates/construct-cli/src/commands.rs`:
```rust
use clap::{Parser, Subcommand};
use construct_config::Config;
use construct_store::SqliteStore;
use construct_core::store::Store;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "entertheconstruct", about = "The Construct — local-first agent runtime")]
struct Cli {
    /// Path to config file.
    #[arg(long, default_value = "construct.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Write a starter construct.toml.
    Init,
    /// Validate the config file.
    ConfigCheck,
    /// Run the vault watcher.
    Watch,
    /// Show watcher/run status summary.
    Status,
    /// List recent runs.
    Runs,
}

pub const SAMPLE_CONFIG: &str = r#"[construct]
name = "The Construct"

[vault]
path = "~/ObsidianVault"
managed_folder = "Construct"

[[agents]]
name = "Scout"
domain = "research"
provider = "ollama"
model = "qwen3.6:27b"
base_url = "http://192.168.1.33:11434"
tools = ["web_search", "web_fetch"]
system_prompt_file = "prompts/scout.md"

[tools.web_search]
backend = "tavily"
api_key_env = "TAVILY_API_KEY"

[[rules]]
match_tag = "theconstruct/research"
agent = "Scout"
pipeline = "research"
"#;

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Init) => {
            if cli.config.exists() {
                println!("{} already exists; not overwriting.", cli.config.display());
            } else {
                std::fs::write(&cli.config, SAMPLE_CONFIG)?;
                println!("Wrote starter config to {}", cli.config.display());
            }
        }
        Some(Command::ConfigCheck) => {
            let cfg = Config::load(&cli.config)?;
            println!("OK: '{}' with {} agent(s), {} rule(s).", cfg.construct.name, cfg.agents.len(), cfg.rules.len());
        }
        Some(Command::Watch) => {
            let cfg = Config::load(&cli.config)?;
            crate::tui::watch_loop::run_watch(cfg).await?;
        }
        Some(Command::Status) | Some(Command::Runs) => {
            let store = SqliteStore::connect("sqlite://construct.db").await?;
            for r in store.list_runs(20).await? {
                println!("{}  {:<10}  {}", r.id, r.status.as_str(), r.note_path);
            }
        }
        None => {
            // No subcommand → launch the TUI.
            let cfg = Config::load(&cli.config).ok();
            crate::tui::run_tui(cfg).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_config_is_valid() {
        let cfg: Config = toml::from_str(SAMPLE_CONFIG).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.agents[0].name, "Scout");
    }
}
```

Note: this references `crate::tui::run_tui`, `crate::tui::watch_loop::run_watch` — defined in Tasks 20–21. Until those exist, temporarily stub them in `tui/mod.rs` so the crate compiles:
```rust
pub mod watch_loop {
    use construct_config::Config;
    pub async fn run_watch(_cfg: Config) -> anyhow::Result<()> { Ok(()) }
}
pub async fn run_tui(_cfg: Option<construct_config::Config>) -> anyhow::Result<()> { Ok(()) }
```
Add `toml` to `[dev-dependencies]` of construct-cli:
```toml
[dev-dependencies]
toml.workspace = true
```

- [ ] **Step 2: Run, expect PASS**
Run: `cargo test -p construct-cli commands`
Expected: PASS (1 test).

- [ ] **Step 3: Manual smoke**
Run: `cargo run -p construct-cli -- init --config /tmp/c.toml && cargo run -p construct-cli -- config-check --config /tmp/c.toml`
Expected: writes config, then prints `OK: 'The Construct' with 1 agent(s), 1 rule(s).`

- [ ] **Step 4: Commit**
```bash
git add crates/construct-cli
git commit -m "feat(cli): init/config-check/watch/status/runs subcommands"
```

---

## Task 20: TUI — dashboard + chat panes

**Files:**
- Modify: `crates/construct-cli/src/tui/mod.rs`
- Create: `crates/construct-cli/src/tui/dashboard.rs`, `src/tui/chat.rs`

This task builds the visible TUI. Logic that can be unit-tested (input handling,
chat state) is factored into pure functions; the render/event loop is exercised
manually.

- [ ] **Step 1: Chat state as a testable unit**

`crates/construct-cli/src/tui/chat.rs`:
```rust
use construct_core::model::{ChatMessage, ChatRequest, ModelProvider};
use std::sync::Arc;

/// Holds the chat transcript and the in-progress input line.
pub struct ChatState {
    pub model: String,
    pub history: Vec<ChatMessage>,
    pub input: String,
}

impl ChatState {
    pub fn new(model: String, system: &str) -> Self {
        ChatState { model, history: vec![ChatMessage::system(system)], input: String::new() }
    }

    pub fn push_char(&mut self, c: char) { self.input.push(c); }
    pub fn backspace(&mut self) { self.input.pop(); }

    /// Take the current input as a user message, clearing the buffer.
    pub fn take_input(&mut self) -> Option<String> {
        let t = self.input.trim().to_string();
        self.input.clear();
        if t.is_empty() { None } else {
            self.history.push(ChatMessage::user(&t));
            Some(t)
        }
    }

    /// Send the conversation to the model and append the reply.
    pub async fn send(&mut self, provider: Arc<dyn ModelProvider>) -> anyhow::Result<()> {
        let req = ChatRequest { model: self.model.clone(), messages: self.history.clone(), tools: vec![] };
        let resp = provider.chat(req).await.map_err(|e| anyhow::anyhow!(e.to_string()))?;
        self.history.push(resp.message);
        Ok(())
    }

    /// Visible (non-system) lines for rendering.
    pub fn visible(&self) -> Vec<&ChatMessage> {
        self.history.iter().filter(|m| !matches!(m.role, construct_core::model::Role::System)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_editing_and_take() {
        let mut s = ChatState::new("m".into(), "sys");
        s.push_char('h'); s.push_char('i'); s.push_char('x'); s.backspace();
        assert_eq!(s.input, "hi");
        assert_eq!(s.take_input().as_deref(), Some("hi"));
        assert!(s.input.is_empty());
        // empty input is ignored
        assert!(s.take_input().is_none());
    }

    #[test]
    fn visible_excludes_system() {
        let mut s = ChatState::new("m".into(), "sys");
        s.push_char('q'); let _ = s.take_input();
        assert_eq!(s.visible().len(), 1); // just the user message
    }
}
```

- [ ] **Step 2: Dashboard view-model as a testable unit**

`crates/construct-cli/src/tui/dashboard.rs`:
```rust
use construct_core::store::RunRecord;

/// Format a run as a single dashboard row. Pure → testable.
pub fn run_row(r: &RunRecord) -> String {
    let name = std::path::Path::new(&r.note_path)
        .file_name().and_then(|s| s.to_str()).unwrap_or(&r.note_path);
    format!("{:<10} {:<8} {}", r.status.as_str(), &r.agent, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use construct_core::types::{RunId, RunStatus};

    #[test]
    fn formats_run_row_with_basename() {
        let r = RunRecord {
            id: RunId("x".into()), rule: "research".into(), agent: "Scout".into(),
            note_path: "/vault/My Topic.md".into(), status: RunStatus::Review, error: None,
        };
        let row = run_row(&r);
        assert!(row.contains("review"));
        assert!(row.contains("Scout"));
        assert!(row.contains("My Topic.md"));
        assert!(!row.contains("/vault/"));
    }
}
```

- [ ] **Step 3: Wire the render/event loop in `tui/mod.rs`**

`crates/construct-cli/src/tui/mod.rs`:
```rust
pub mod chat;
pub mod dashboard;
pub mod watch_loop;

use crate::theme::Theme;
use chat::ChatState;
use construct_config::Config;
use construct_model_ollama::OllamaProvider;
use crossterm::event::{self, Event, KeyCode};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use std::sync::Arc;
use std::time::Duration;

/// Launch the TUI: header, runs panel (left), chat (right).
pub async fn run_tui(cfg: Option<Config>) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let (model, base_url, system) = match &cfg {
        Some(c) if !c.agents.is_empty() => (c.agents[0].model.clone(), c.agents[0].base_url.clone(), format!("You are {}.", c.agents[0].name)),
        _ => ("llama3.1".into(), "http://localhost:11434".into(), "You are a helpful assistant.".into()),
    };
    let provider: Arc<dyn construct_core::model::ModelProvider> = Arc::new(OllamaProvider::new(base_url));
    let mut chat = ChatState::new(model, &system);

    loop {
        terminal.draw(|f| draw(f, &chat))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Esc => break,
                    KeyCode::Char(c) => chat.push_char(c),
                    KeyCode::Backspace => chat.backspace(),
                    KeyCode::Enter => {
                        if chat.take_input().is_some() {
                            // Blocking send for v1 simplicity; spinner/async streaming is a later slice.
                            chat.send(provider.clone()).await.ok();
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    ratatui::restore();
    Ok(())
}

fn draw(f: &mut Frame, chat: &ChatState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(3)])
        .split(f.area());

    let header = Paragraph::new("  The Construct ").style(Theme::header());
    f.render_widget(header, chunks[0]);

    let transcript: Vec<Line> = chat.visible().iter()
        .map(|m| Line::from(format!("{:?}: {}", m.role, m.content)).style(Theme::body()))
        .collect();
    let body = Paragraph::new(transcript)
        .block(Block::default().borders(Borders::ALL).title("Chat").border_style(Theme::accent()));
    f.render_widget(body, chunks[1]);

    let input = Paragraph::new(chat.input.as_str())
        .style(Theme::body())
        .block(Block::default().borders(Borders::ALL).title("Type (Esc to quit)").border_style(Theme::accent()));
    f.render_widget(input, chunks[2]);
}
```

- [ ] **Step 4: Run unit tests, expect PASS**
Run: `cargo test -p construct-cli`
Expected: PASS (theme + chat + dashboard tests).

- [ ] **Step 5: Manual smoke** (requires a terminal + running Ollama)
Run: `cargo run -p construct-cli -- --config /tmp/c.toml`
Expected: TUI opens with earthy theme; typing + Enter gets a reply from the local model; Esc exits.

- [ ] **Step 6: Commit**
```bash
git add crates/construct-cli
git commit -m "feat(cli): TUI with chat pane and dashboard view-models"
```

---

## Task 21: Watch loop (wires watcher → orchestrator)

**Files:**
- Modify: `crates/construct-cli/src/tui/watch_loop.rs`

- [ ] **Step 1: Implement `run_watch` assembling all the real pieces**

`crates/construct-cli/src/tui/watch_loop.rs`:
```rust
use construct_config::Config;
use construct_engine::orchestrator::Orchestrator;
use construct_engine::rules::known_tags;
use construct_core::model::ModelProvider;
use construct_core::store::Store;
use construct_core::tool::Tool;
use construct_model_ollama::OllamaProvider;
use construct_obsidian::watcher::{watch, VaultEvent};
use construct_store::SqliteStore;
use construct_tools::{WebFetch, WebSearch};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Build an orchestrator from config + start the watcher; process events forever.
pub async fn run_watch(cfg: Config) -> anyhow::Result<()> {
    let agent = cfg.agents.first().cloned()
        .ok_or_else(|| anyhow::anyhow!("config has no agents"))?;
    let rule = cfg.rules.first().cloned()
        .ok_or_else(|| anyhow::anyhow!("config has no rules"))?;

    let store: Arc<dyn Store> = Arc::new(SqliteStore::connect("sqlite://construct.db").await?);
    let provider: Arc<dyn ModelProvider> = Arc::new(OllamaProvider::new(agent.base_url.clone()));

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    if agent.tools.iter().any(|t| t == "web_search") {
        if let Some(ws) = &cfg.tools.web_search {
            let key = std::env::var(&ws.api_key_env).unwrap_or_default();
            tools.insert("web_search".into(), Arc::new(WebSearch::tavily(key)));
        }
    }
    if agent.tools.iter().any(|t| t == "web_fetch") {
        tools.insert("web_fetch".into(), Arc::new(WebFetch::new()));
    }

    let system_prompt = agent.system_prompt_file.as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| format!("You are {}, a meticulous web research agent. Always answer with strict JSON: {{summary, findings[], sources[{{title,url}}]}}.", agent.name));

    let orchestrator = Arc::new(Orchestrator {
        store, provider, tools,
        model: agent.model.clone(),
        agent: agent.name.clone(),
        rule: rule.pipeline.clone(),
        system_prompt,
        max_iterations: 8,
        done_tag: Some("theconstruct/done".into()),
    });

    // Crash recovery: re-trigger any run left mid-flight before we start watching.
    if let Err(e) = orchestrator.reconcile().await {
        tracing::warn!("reconcile failed: {e}");
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<VaultEvent>();
    let vault = shellexpand_tilde(&cfg.vault.path);
    let _debouncer = watch(vault.into(), known_tags(&cfg), "construct_status".into(), tx);

    println!("The Construct is watching. Tag a note with #{} to begin.", rule.match_tag);
    while let Some(event) = rx.recv().await {
        let o = orchestrator.clone();
        tokio::spawn(async move {
            if let Err(e) = o.handle(event).await {
                tracing::error!("handler error: {e}");
            }
        });
    }
    Ok(())
}

/// Minimal `~` expansion (avoids an extra dependency).
fn shellexpand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tilde() {
        std::env::set_var("HOME", "/home/x");
        assert_eq!(shellexpand_tilde("~/vault"), "/home/x/vault");
        assert_eq!(shellexpand_tilde("/abs"), "/abs");
    }
}
```

- [ ] **Step 2: Run, expect PASS**
Run: `cargo test -p construct-cli watch_loop`
Expected: PASS (1 test).

- [ ] **Step 3: Build the whole workspace**
Run: `cargo build`
Expected: clean build of all crates + the `entertheconstruct` binary.

- [ ] **Step 4: Commit**
```bash
git add crates/construct-cli
git commit -m "feat(cli): watch loop wiring watcher to orchestrator"
```

---

## Task 22: Workspace-wide verification + README

**Files:**
- Create: `README.md`
- Create: `prompts/scout.md`

- [ ] **Step 1: Add a default agent prompt**

`prompts/scout.md`:
```markdown
You are Scout, a meticulous web research agent for The Construct.

Given a topic, use the `web_search` tool to find sources and `web_fetch` to read
the most relevant ones. Then answer with STRICT JSON only, no prose, matching:

{
  "summary": "2-4 sentence synthesis",
  "findings": ["concise factual finding", "..."],
  "sources": [{"title": "Page title", "url": "https://..."}]
}
```

- [ ] **Step 2: README with install + usage**

`README.md`:
```markdown
# The Construct

A local-first, deterministic-first agent runtime. Slice 1: watch an Obsidian
vault and turn a `#theconstruct/research` note into a reviewable, web-researched
draft using your local Ollama model.

## Install
```bash
cargo install --path crates/construct-cli
```
This installs the `entertheconstruct` binary.

## Quick start
```bash
entertheconstruct init                 # write construct.toml
$EDITOR construct.toml                  # set vault.path, model, base_url
export TAVILY_API_KEY=...               # web search key
entertheconstruct config-check
entertheconstruct watch                 # start watching
```
Create a note in your vault, add `#theconstruct/research`, save. The agent
researches, writes a draft into the note, and sets `construct_status: review`.
Change it to `accepted` or `rejected` in frontmatter to finalize.

Run `entertheconstruct` with no arguments to open the TUI (dashboard + chat).

## Architecture
Deterministic shell, single agentic step. See
`docs/superpowers/specs/2026-05-29-the-construct-slice-1-design.md`.
```

- [ ] **Step 3: Full test + lint sweep**
Run:
```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```
Expected: all tests pass; no clippy warnings; formatting clean. Fix anything that fails.

- [ ] **Step 4: Commit**
```bash
git add README.md prompts/
git commit -m "docs: add README and default Scout prompt; finalize Slice 1"
```

---

## Self-Review

**1. Spec coverage** (each spec §/requirement → task):
- Workspace + traits (§3, §7) → Tasks 1–3 ✓
- Config TOML + validation (§5, FR10) → Task 4 ✓
- SQLite store + resume state (§8, FR7) → Task 5 ✓
- Ollama ModelProvider; frontier via trait (§3.1, FR3, FR13) → Tasks 3, 6 ✓
- web_search/web_fetch tools; SearXNG via trait (FR3, FR13) → Tasks 7–8 ✓
- Frontmatter, managed block, watcher (FR1, FR5) → Tasks 9–11 ✓
- Rule routing (FR2) → Task 12 ✓
- Agentic loop bounded + evidence capture (FR3) → Tasks 13–14 ✓
- Deterministic gate: shape **+ source grounding** (FR4, design §4.1) → Task 15 ✓
- Frontmatter contract / ignore-self (design §4.2) → Tasks 9, 11 (classify), 16 (keys) ✓
- Crash recovery / resume (design §4.3) → Task 5 (`runs_with_status`), Task 17 (`reconcile`), Task 21 (startup call) ✓
- Pipeline state machine, deterministic mutations only, **no note-move** (§4, FR5, FR6, FR11) → Tasks 16–17 ✓
- Accept/reject finalize (FR6) → Tasks 16–17 ✓
- Error to frontmatter, never crash (FR12) → Task 17 (`fail`) + Task 21 (spawn isolates) ✓
- CLI subcommands + entrypoint (FR8) → Tasks 18–19, 21 ✓
- TUI dashboard + chat (FR9) → Task 20 ✓
- Earthy theme → Task 18 ✓
- End-to-end mock test (§10) → Task 17 ✓

**2. Placeholder scan:** The only `// filled in Task N` markers are explicit, ordered module stubs needed so each crate compiles before its later task — each is replaced by a named later task, and the stub contents are given. No requirement is left as TBD.

**3. Type consistency:** `RunStatus`, `RunRecord`, `RunId`, `ResearchResult`/`Source`, `ChatMessage`/`ToolCall`/`ChatRequest`/`ChatResponse`, `ToolSpec`, `Store` method names (`create_run`, `update_status`, `run_for_note`, `append_event`, `list_runs`, `runs_with_status`), `STATUS_KEY`/`RUN_KEY`, and the `apply_*` stage fns are used with identical signatures across Tasks 2–21. The agent loop returns `LoopOutput { content, evidence }` (Task 14) and the gate is `validate(raw, evidence)` (Task 15); the orchestrator (Task 17) calls both consistently. The frontmatter status field name is `construct_status` consistently (the design's `construct.status` nested form is flattened to the single key `construct_status` for simpler, robust YAML editing — a deliberate, consistent choice matching design §4.2).

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-29-the-construct-slice-1.md`. Two execution options:

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
