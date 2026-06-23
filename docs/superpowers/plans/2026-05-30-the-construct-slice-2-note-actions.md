# The Construct — Slice 2: Note Actions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three local-model note actions — Summarize (auto), Tag (auto), and Organize (review-gated move) — on top of a refactor that lets the orchestrator dispatch any tag to any built-in pipeline.

**Architecture:** Phase 0 generalizes `Orchestrator` from a single hardcoded research flow to dispatch-by-`rule.pipeline`, preserving research behavior. Phases 1–3 add each action as its own pipeline: pure deterministic transforms + a per-action deterministic gate, with the agent returning JSON only. Summarize/tag auto-apply; organize proposes a move and waits for a frontmatter `accepted`/`rejected` decision. Multiple actions on one note are serialized via a per-note async lock.

**Tech Stack:** Rust, tokio, serde/serde_json/serde_yaml, sqlx (SQLite). Reuses Slice 1 crates: construct-core, construct-config, construct-store, construct-model-ollama, construct-obsidian, construct-engine, construct-cli.

---

## Conventions for every task

- TDD: write the failing test, run it (confirm failure), implement minimally, run it (confirm pass), commit.
- Run one crate's tests: `cargo test -p <crate>`.
- Commit with: `git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "<msg>\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"`.
- After each task run `cargo build` at the workspace root.
- Do NOT push unless explicitly asked; the controller handles pushes.
- Ground truth before/after each task is `git log` + `cargo test`, never agent chatter.

---

## File Structure (what changes)

```
crates/construct-obsidian/src/
  block.rs            # MODIFY: generalize markers (name param), keep research back-compat
  frontmatter.rs      # MODIFY: tags-list read/merge helpers
  vault.rs            # CREATE: scan folders + gather existing tags
crates/construct-config/src/
  lib.rs              # MODIFY: ActionsCfg structs; validate known pipelines
crates/construct-engine/src/
  gate.rs             # MODIFY: add validate_summary / validate_tags / validate_organize + shared extract_json
  actions.rs          # CREATE: typed agent-output structs (SummaryOut, TagsOut, OrganizeOut)
  pipelines/
    mod.rs            # CREATE: Pipeline kind enum + shared claim/finalize helpers (moved from pipeline.rs)
    research.rs       # CREATE: research transforms (moved from pipeline.rs, unchanged behavior)
    summarize.rs      # CREATE
    tag.rs            # CREATE
    organize.rs       # CREATE
  pipeline.rs         # MODIFY: re-export from pipelines/ for back-compat (or delete + update imports)
  orchestrator.rs     # MODIFY: dispatch by pipeline name; generalize reconcile
  lib.rs              # MODIFY: module wiring
crates/construct-cli/src/
  commands.rs         # MODIFY: SAMPLE_CONFIG gains Librarian agent + 3 rules
  tui/watch_loop.rs   # MODIFY: build Orchestrator with full config; per-note serialization
prompts/
  librarian.md        # CREATE
```

---

# PHASE 0 — Multi-pipeline dispatch refactor

Goal: research keeps working; the engine can route by `rule.pipeline`. No new user-facing behavior.

## Task 0.1: Config — actions settings + pipeline validation

**Files:**
- Modify: `crates/construct-config/src/lib.rs`

- [ ] **Step 1: Write failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/construct-config/src/lib.rs`:
```rust
    #[test]
    fn rejects_unknown_pipeline() {
        let toml = r#"
[construct]
name = "C"
[vault]
path = "/v"
[[agents]]
name = "A"
domain = "d"
provider = "ollama"
model = "m"
base_url = "http://localhost:11434"
[[rules]]
match_tag = "theconstruct/bogus"
agent = "A"
pipeline = "bogus"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn accepts_known_pipelines_and_actions_defaults() {
        let toml = r#"
[construct]
name = "C"
[vault]
path = "/v"
[[agents]]
name = "Lib"
domain = "notes"
provider = "ollama"
model = "m"
base_url = "http://localhost:11434"
[[rules]]
match_tag = "theconstruct/tag"
agent = "Lib"
pipeline = "tag"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.actions.tag.max_tags, 8); // default
    }
```

- [ ] **Step 2: Run, expect FAIL** (`cfg.actions` field and pipeline check don't exist yet)
Run: `cargo test -p construct-config`
Expected: compile error / FAIL.

- [ ] **Step 3: Implement**

Add the actions structs after `WebSearchCfg` (around line 58) in `crates/construct-config/src/lib.rs`:
```rust
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct ActionsCfg {
    #[serde(default)]
    pub tag: TagActionCfg,
    #[serde(default)]
    pub organize: OrganizeActionCfg,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TagActionCfg {
    #[serde(default = "default_max_tags")]
    pub max_tags: usize,
}
impl Default for TagActionCfg {
    fn default() -> Self {
        TagActionCfg { max_tags: default_max_tags() }
    }
}
fn default_max_tags() -> usize {
    8
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub struct OrganizeActionCfg {
    #[serde(default)]
    pub exclude_dirs: Vec<String>,
}

/// The set of built-in pipeline names this binary knows how to run.
pub const KNOWN_PIPELINES: &[&str] = &["research", "summarize", "tag", "organize"];
```

Add the field to `Config` (after `tools`):
```rust
    #[serde(default)]
    pub actions: ActionsCfg,
```

Extend `validate` (inside the existing `for rule in &self.rules` loop, after the agent check):
```rust
            if !KNOWN_PIPELINES.contains(&rule.pipeline.as_str()) {
                return Err(ConfigError::Validation(format!(
                    "rule for tag '{}' names unknown pipeline '{}' (known: {:?})",
                    rule.match_tag, rule.pipeline, KNOWN_PIPELINES
                )));
            }
```

- [ ] **Step 4: Run, expect PASS** (and existing config tests still pass)
Run: `cargo test -p construct-config`

- [ ] **Step 5: Commit**
```bash
git add crates/construct-config
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(config): actions settings + known-pipeline validation

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 0.2: Generalize managed blocks to named markers

**Files:**
- Modify: `crates/construct-obsidian/src/block.rs`

Goal: support multiple managed blocks (research, summary) by name, keeping the
existing `upsert_block`/`remove_block` working (they delegate to the named form
with `"research"`).

- [ ] **Step 1: Write failing tests**

Add to the tests module in `crates/construct-obsidian/src/block.rs`:
```rust
    #[test]
    fn named_blocks_are_independent() {
        let b = upsert_named("body", "summary", "SUM");
        let b = upsert_named(&b, "research", "RES");
        assert!(b.contains("construct:summary:start"));
        assert!(b.contains("construct:research:start"));
        assert!(b.contains("SUM"));
        assert!(b.contains("RES"));
        // replacing summary leaves research intact
        let b2 = upsert_named(&b, "summary", "SUM2");
        assert!(b2.contains("SUM2"));
        assert!(!b2.contains("SUM\n"));
        assert!(b2.contains("RES"));
    }

    #[test]
    fn upsert_at_top_places_block_first() {
        let out = upsert_named_at_top("hello body", "summary", "TLDR");
        assert!(out.trim_start().starts_with("<!-- construct:summary:start -->"));
        assert!(out.contains("hello body"));
    }

    #[test]
    fn research_back_compat() {
        let out = upsert_block("hello", "RESULT");
        assert!(out.contains("construct:research:start"));
        assert!(out.contains("RESULT"));
    }
```

- [ ] **Step 2: Run, expect FAIL**
Run: `cargo test -p construct-obsidian block`
Expected: FAIL (`upsert_named` not found).

- [ ] **Step 3: Implement — replace the whole non-test portion of `block.rs`**
```rust
fn markers(name: &str) -> (String, String) {
    (
        format!("<!-- construct:{name}:start -->"),
        format!("<!-- construct:{name}:end -->"),
    )
}

/// Insert or replace a named managed block at the END of the body.
pub fn upsert_named(body: &str, name: &str, content: &str) -> String {
    let (start, end) = markers(name);
    let block = format!("{start}\n{content}\n{end}");
    if let (Some(s), Some(e)) = (body.find(&start), body.find(&end)) {
        let mut out = String::new();
        out.push_str(&body[..s]);
        out.push_str(&block);
        out.push_str(&body[e + end.len()..]);
        out
    } else {
        let sep = if body.ends_with('\n') || body.is_empty() {
            ""
        } else {
            "\n"
        };
        format!("{body}{sep}\n{block}\n")
    }
}

/// Insert or replace a named managed block at the TOP of the body.
pub fn upsert_named_at_top(body: &str, name: &str, content: &str) -> String {
    let (start, end) = markers(name);
    let block = format!("{start}\n{content}\n{end}");
    if let (Some(s), Some(e)) = (body.find(&start), body.find(&end)) {
        // Replace in place.
        let mut out = String::new();
        out.push_str(&body[..s]);
        out.push_str(&block);
        out.push_str(&body[e + end.len()..]);
        out
    } else {
        let sep = if body.is_empty() { "" } else { "\n\n" };
        format!("{block}{sep}{body}")
    }
}

/// Remove a named managed block entirely.
pub fn remove_named(body: &str, name: &str) -> String {
    let (start, end) = markers(name);
    if let (Some(s), Some(e)) = (body.find(&start), body.find(&end)) {
        let mut out = String::new();
        out.push_str(body[..s].trim_end());
        out.push_str(&body[e + end.len()..]);
        out
    } else {
        body.to_string()
    }
}

/// Back-compat: research block at end of body.
pub fn upsert_block(body: &str, content: &str) -> String {
    upsert_named(body, "research", content)
}

/// Back-compat: remove the research block.
pub fn remove_block(body: &str) -> String {
    remove_named(body, "research")
}
```
Keep the existing 3 research tests (`inserts_when_absent`, `replaces_when_present`, `removes_block`) — they still pass via back-compat.

- [ ] **Step 4: Run, expect PASS**
Run: `cargo test -p construct-obsidian block`

- [ ] **Step 5: Commit**
```bash
git add crates/construct-obsidian
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(obsidian): named managed blocks (top/bottom) with research back-compat

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 0.3: Split pipeline.rs into a pipelines module (research unchanged)

**Files:**
- Create: `crates/construct-engine/src/pipelines/mod.rs`, `crates/construct-engine/src/pipelines/research.rs`
- Modify: `crates/construct-engine/src/lib.rs`, `crates/construct-engine/src/pipeline.rs`

Goal: move the existing research transforms into `pipelines::research` and add a
`PipelineKind` enum, WITHOUT changing behavior. Keep `pipeline.rs` as a thin
re-export so existing imports (`crate::pipeline::{...}`, `STATUS_KEY`) still work.

- [ ] **Step 1: Create `pipelines/mod.rs`**
```rust
pub mod research;

/// Which built-in pipeline a rule selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineKind {
    Research,
    Summarize,
    Tag,
    Organize,
}

impl PipelineKind {
    pub fn from_name(name: &str) -> Option<PipelineKind> {
        match name {
            "research" => Some(PipelineKind::Research),
            "summarize" => Some(PipelineKind::Summarize),
            "tag" => Some(PipelineKind::Tag),
            "organize" => Some(PipelineKind::Organize),
            _ => None,
        }
    }
    /// Auto-apply pipelines finish without a human review step.
    pub fn is_auto_apply(&self) -> bool {
        matches!(self, PipelineKind::Summarize | PipelineKind::Tag)
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
    fn claim_sets_status_and_run() {
        use construct_obsidian::frontmatter::Note;
        let out = apply_claim("body", "run-1");
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("queued"));
        assert_eq!(note.get_str(RUN_KEY).as_deref(), Some("run-1"));
    }
}
```

- [ ] **Step 2: Create `pipelines/research.rs`** (move the research-specific transforms verbatim)
```rust
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
        let accepted = text.replace("review", "accepted");
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
```

- [ ] **Step 3: Replace `pipeline.rs` with a back-compat re-export**
```rust
//! Back-compat facade. Real implementations live in `crate::pipelines`.
pub use crate::pipelines::research::{apply_accept, apply_reject, apply_write_back, render_result};
pub use crate::pipelines::{apply_claim, RUN_KEY, STATUS_KEY};
```

- [ ] **Step 4: Wire modules in `lib.rs`** — add `pub mod pipelines;` (keep `pub mod pipeline;`).

- [ ] **Step 5: Run, expect PASS** (research transforms + orchestrator tests unchanged)
Run: `cargo test -p construct-engine`
Expected: all existing engine tests pass (orchestrator imports via `crate::pipeline::*` still resolve).

- [ ] **Step 6: Commit**
```bash
git add crates/construct-engine
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "refactor(engine): split pipelines module + PipelineKind; research unchanged

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 0.4: Orchestrator dispatches by pipeline (research still the only one wired)

**Files:**
- Modify: `crates/construct-engine/src/orchestrator.rs`

Goal: add a `pipeline: PipelineKind` field; `handle_tagged` matches on it; only
`Research` is implemented (others `todo!`-free: return a clear "not yet wired"
error routed through `fail`). This keeps the refactor green; Phases 1–3 fill the arms.

- [ ] **Step 1: Write failing test**

Add to orchestrator tests:
```rust
    #[tokio::test]
    async fn unimplemented_pipeline_sets_error() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("n.md");
        std::fs::write(&note_path, "Body #theconstruct/tag").unwrap();
        let mut o = orch(Arc::new(search_then_answer())).await;
        o.pipeline = crate::pipelines::PipelineKind::Tag; // not wired until Phase 2
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/tag".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: error"));
    }
```
And update the `orch(...)` helper to set `pipeline: crate::pipelines::PipelineKind::Research`.

- [ ] **Step 2: Run, expect FAIL** (`pipeline` field missing)
Run: `cargo test -p construct-engine orchestrator`

- [ ] **Step 3: Implement**

Add to the `Orchestrator` struct (after `rule`):
```rust
    pub pipeline: crate::pipelines::PipelineKind,
```

In `handle_tagged`, replace the research-specific body (steps 2–5, the part after `claim`) with a dispatch. Keep `claim` shared, then:
```rust
        // Dispatch by pipeline.
        use crate::pipelines::PipelineKind;
        match self.pipeline {
            PipelineKind::Research => self.run_research(&run_id, path, &original).await,
            _ => {
                self.fail(
                    &run_id,
                    path,
                    &format!("pipeline {:?} not wired yet", self.pipeline),
                )
                .await
            }
        }
```
Move the existing research steps (researching status → run_loop → gate → write_back → review) into a new method `run_research(&self, run_id: &RunId, path: &Path, original: &str) -> anyhow::Result<()>` containing exactly the current logic (lines that set `Researching`, build the prompt, run the loop, gate, write_back, set `Review`). No behavior change.

Update `orch(...)` test helper to include `pipeline: crate::pipelines::PipelineKind::Research,`.

- [ ] **Step 4: Run, expect PASS** (all 4 prior orchestrator tests + the new one)
Run: `cargo test -p construct-engine`

- [ ] **Step 5: Commit**
```bash
git add crates/construct-engine
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "refactor(engine): orchestrator dispatch by PipelineKind (research wired)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 0.5: Wire the watch loop to resolve pipeline per rule

**Files:**
- Modify: `crates/construct-cli/src/tui/watch_loop.rs`

Goal: build one `Orchestrator` per configured rule (keyed by tag), so the watch
loop can route an event to the right one. For Phase 0 this still only exercises
research, but the wiring is general.

- [ ] **Step 1: Implement** — change `run_watch` to build a map of orchestrators.

Replace the single-orchestrator construction with one per rule:
```rust
    use std::collections::HashMap as Map;
    let mut orchestrators: Map<String, Arc<Orchestrator>> = Map::new();
    for rule in &cfg.rules {
        let agent = cfg
            .agent(&rule.agent)
            .ok_or_else(|| anyhow::anyhow!("rule references unknown agent"))?
            .clone();
        let provider: Arc<dyn ModelProvider> =
            Arc::new(OllamaProvider::new(agent.base_url.clone()));
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
        let system_prompt = agent
            .system_prompt_file
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_else(|| format!("You are {}.", agent.name));
        let kind = construct_engine::pipelines::PipelineKind::from_name(&rule.pipeline)
            .ok_or_else(|| anyhow::anyhow!("unknown pipeline {}", rule.pipeline))?;
        let orchestrator = Arc::new(Orchestrator {
            store: store.clone(),
            provider,
            tools,
            model: agent.model.clone(),
            agent: agent.name.clone(),
            rule: rule.pipeline.clone(),
            pipeline: kind,
            system_prompt,
            max_iterations: 8,
            done_tag: Some("theconstruct/done".into()),
            // Phase 1+ fields (actions cfg, vault path) added in later tasks.
        });
        orchestrators.insert(rule.match_tag.clone(), orchestrator);
    }
```
And in the event loop, route by tag:
```rust
    while let Some(event) = rx.recv().await {
        let tag = match &event {
            VaultEvent::NoteTagged { tag, .. } => tag.clone(),
            VaultEvent::StatusChanged { path, .. } => {
                // find the orchestrator whose run owns this note; for v2 just try all
                // (decision events are cheap and idempotent). Use the first rule.
                // Simpler: broadcast to all orchestrators; each no-ops if not its run.
                String::new()
            }
        };
        // Route NoteTagged to the matching orchestrator; broadcast StatusChanged.
        match &event {
            VaultEvent::NoteTagged { .. } => {
                if let Some(o) = orchestrators.get(&tag) {
                    let o = o.clone();
                    tokio::spawn(async move {
                        if let Err(e) = o.handle(event).await {
                            tracing::error!("handler error: {e}");
                        }
                    });
                }
            }
            VaultEvent::StatusChanged { .. } => {
                for o in orchestrators.values() {
                    let o = o.clone();
                    let ev = event.clone();
                    tokio::spawn(async move {
                        let _ = o.handle(ev).await;
                    });
                }
            }
        }
    }
```
NOTE: `store` must be built once before the loop (it already is). The `Orchestrator` struct gains `actions`/`vault_path` fields in later tasks; when those are added, update this constructor accordingly (the task that adds a field updates this call site in the same task).

- [ ] **Step 2: Run, expect PASS** (the `expands_tilde` test still passes; build compiles)
Run: `cargo build && cargo test -p construct-cli`

- [ ] **Step 3: Manual smoke (optional, offline):** `cargo run -p construct-cli -- config-check` against a config with multiple rules → prints agent/rule counts.

- [ ] **Step 4: Commit**
```bash
git add crates/construct-cli
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(cli): build per-rule orchestrators and route events by tag

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

# PHASE 1 — Summarize (auto-apply)

## Task 1.1: Summarize output type + gate

**Files:**
- Create: `crates/construct-engine/src/actions.rs`
- Modify: `crates/construct-engine/src/gate.rs`, `crates/construct-engine/src/lib.rs`

- [ ] **Step 1: Create `actions.rs` with the typed outputs**
```rust
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SummaryOut {
    pub tldr: String,
    #[serde(default)]
    pub action_items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TagsOut {
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OrganizeOut {
    pub destination: String,
    #[serde(default)]
    pub reason: String,
}
```
Add `pub mod actions;` to `lib.rs`.

- [ ] **Step 2: Write failing gate tests** — add to `gate.rs` tests:
```rust
    #[test]
    fn summary_gate_accepts_and_rejects() {
        let ok = r#"{"tldr":"Short summary","action_items":["do x"]}"#;
        let s = validate_summary(ok).unwrap();
        assert_eq!(s.tldr, "Short summary");
        assert_eq!(s.action_items.len(), 1);

        let empty = r#"{"tldr":"   ","action_items":[]}"#;
        assert!(matches!(validate_summary(empty), Err(GateError::Invalid(_))));
        assert!(matches!(validate_summary("nope"), Err(GateError::NotJson(_))));
    }
```

- [ ] **Step 3: Run, expect FAIL**
Run: `cargo test -p construct-engine gate`

- [ ] **Step 4: Implement** — make `extract_json` reusable and add `validate_summary`.

In `gate.rs`, ensure `extract_json` is the existing balanced-brace scanner (already present from Slice 1). Add:
```rust
use crate::actions::SummaryOut;

/// Validate a summarize action's output: valid JSON, non-empty tldr.
pub fn validate_summary(raw: &str) -> Result<SummaryOut, GateError> {
    let slice = extract_json(raw).ok_or_else(|| GateError::NotJson("no JSON object found".into()))?;
    let out: SummaryOut =
        serde_json::from_str(slice).map_err(|e| GateError::NotJson(e.to_string()))?;
    if out.tldr.trim().is_empty() {
        return Err(GateError::Invalid("tldr is empty".into()));
    }
    Ok(out)
}
```

- [ ] **Step 5: Run, expect PASS**
Run: `cargo test -p construct-engine gate`

- [ ] **Step 6: Commit**
```bash
git add crates/construct-engine
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(engine): action output types + summarize gate

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 1.2: Summarize transform (managed block at top)

**Files:**
- Create: `crates/construct-engine/src/pipelines/summarize.rs`
- Modify: `crates/construct-engine/src/pipelines/mod.rs`

- [ ] **Step 1: Write failing tests** — `pipelines/summarize.rs`:
```rust
use super::{RUN_KEY, STATUS_KEY};
use crate::actions::SummaryOut;
use construct_obsidian::block::upsert_named_at_top;
use construct_obsidian::frontmatter::Note;

/// Render the summary callout block.
pub fn render_summary(s: &SummaryOut) -> String {
    let mut out = String::from("> [!summary] TL;DR\n");
    for line in s.tldr.lines() {
        out.push_str(&format!("> {line}\n"));
    }
    if !s.action_items.is_empty() {
        out.push_str(">\n> **Action items**\n");
        for item in &s.action_items {
            out.push_str(&format!("> - [ ] {item}\n"));
        }
    }
    out
}

/// Apply summarize: insert/replace the summary block at the top, set status=done.
pub fn apply_summary(text: &str, s: &SummaryOut, done_tag: Option<&str>) -> String {
    let mut note = Note::parse(text);
    note.body = upsert_named_at_top(&note.body, "summary", &render_summary(s));
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

    fn out() -> SummaryOut {
        SummaryOut {
            tldr: "This note is about X.".into(),
            action_items: vec!["Email Bob".into(), "Book room".into()],
        }
    }

    #[test]
    fn inserts_block_at_top_and_done() {
        let text = super::super::apply_claim("Original body here.", "r1");
        let applied = apply_summary(&text, &out(), Some("theconstruct/done"));
        let note = Note::parse(&applied);
        assert!(note.body.trim_start().starts_with("<!-- construct:summary:start -->"));
        assert!(note.body.contains("TL;DR"));
        assert!(note.body.contains("- [ ] Email Bob"));
        assert!(note.body.contains("Original body here."));
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("done"));
        assert!(note.get_str(RUN_KEY).is_none());
        assert!(note.body.contains("#theconstruct/done"));
    }

    #[test]
    fn rerun_replaces_single_block() {
        let text = super::super::apply_claim("body", "r1");
        let once = apply_summary(&text, &out(), None);
        let twice = apply_summary(&once, &SummaryOut { tldr: "New".into(), action_items: vec![] }, None);
        assert_eq!(twice.matches("construct:summary:start").count(), 1);
        assert!(twice.contains("New"));
    }
}
```
Add `pub mod summarize;` to `pipelines/mod.rs`.

- [ ] **Step 2: Run, expect FAIL** then **PASS** after the module compiles.
Run: `cargo test -p construct-engine summarize`
Expected: 2 tests pass.

- [ ] **Step 3: Commit**
```bash
git add crates/construct-engine
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(engine): summarize transform (TL;DR managed block at top)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 1.3: Wire summarize into the orchestrator

**Files:**
- Modify: `crates/construct-engine/src/orchestrator.rs`

- [ ] **Step 1: Write failing end-to-end test** — add to orchestrator tests:
```rust
    fn summary_model() -> ScriptedModel {
        ScriptedModel::new(vec![ChatResponse {
            message: ChatMessage::assistant(
                r#"{"tldr":"It is about X.","action_items":["do y"]}"#,
            ),
        }])
    }

    #[tokio::test]
    async fn summarize_auto_applies_to_done() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("n.md");
        std::fs::write(&note_path, "Long note body #theconstruct/summarize").unwrap();
        let mut o = orch(Arc::new(summary_model())).await;
        o.pipeline = crate::pipelines::PipelineKind::Summarize;
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/summarize".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: done"));
        assert!(after.contains("TL;DR"));
        assert!(after.contains("- [ ] do y"));
    }
```

- [ ] **Step 2: Run, expect FAIL** (Summarize arm still routes to "not wired" error)
Run: `cargo test -p construct-engine summarize_auto_applies`

- [ ] **Step 3: Implement** — add a `run_summarize` method and wire the arm.

Add the method:
```rust
    async fn run_summarize(
        &self,
        run_id: &RunId,
        path: &Path,
        original: &str,
    ) -> anyhow::Result<()> {
        self.store
            .update_status(run_id, RunStatus::Running, None)
            .await?;
        let note = Note::parse(original);
        let user_prompt = format!(
            "Summarize the following note. Return STRICT JSON only: \
             {{\"tldr\": string, \"action_items\": [string]}}.\n\n{}",
            note.body
        );
        let messages = vec![
            ChatMessage::system(&self.system_prompt),
            ChatMessage::user(user_prompt),
        ];
        let out = match run_loop(
            self.provider.as_ref(),
            &self.tools,
            messages,
            &LoopConfig {
                model: self.model.clone(),
                max_iterations: self.max_iterations,
            },
        )
        .await
        {
            Ok(r) => r,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let summary = match crate::gate::validate_summary(&out.content) {
            Ok(s) => s,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let current = std::fs::read_to_string(path)?;
        let applied = crate::pipelines::summarize::apply_summary(
            &current,
            &summary,
            self.done_tag.as_deref(),
        );
        std::fs::write(path, applied)?;
        self.store
            .update_status(run_id, RunStatus::Done, None)
            .await?;
        self.store
            .append_event(run_id, "summarize", "done", serde_json::json!({}))
            .await?;
        Ok(())
    }
```
Wire the dispatch arm:
```rust
            PipelineKind::Summarize => self.run_summarize(&run_id, path, &original).await,
```
This requires a `RunStatus::Running` variant — add it in the next task's note if not present. (See Task 1.4.)

- [ ] **Step 4: Run, expect PASS** (after Task 1.4 adds `Running`)
Run: `cargo test -p construct-engine`

- [ ] **Step 5: Commit**
```bash
git add crates/construct-engine
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(engine): wire summarize pipeline end-to-end

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 1.4: Add `RunStatus::Running` + store mapping

**Files:**
- Modify: `crates/construct-core/src/types.rs`, `crates/construct-store/src/lib.rs`

NOTE: Do this BEFORE Task 1.3 compiles. (Ordering: 1.1 → 1.2 → 1.4 → 1.3.)

- [ ] **Step 1: Write failing test** — in `types.rs` tests:
```rust
    #[test]
    fn running_status_round_trips() {
        assert_eq!(RunStatus::Running.as_str(), "running");
        let j = serde_json::to_string(&RunStatus::Running).unwrap();
        assert_eq!(serde_json::from_str::<RunStatus>(&j).unwrap(), RunStatus::Running);
    }
```

- [ ] **Step 2: Run, expect FAIL**
Run: `cargo test -p construct-core running_status`

- [ ] **Step 3: Implement** — add `Running` to the `RunStatus` enum and its `as_str`:
```rust
    Running,
```
In `as_str`: `RunStatus::Running => "running",`.
In `construct-store/src/lib.rs` `map_status`: add `"running" => RunStatus::Running,`.

- [ ] **Step 4: Run, expect PASS**
Run: `cargo test -p construct-core && cargo test -p construct-store`

- [ ] **Step 5: Commit**
```bash
git add crates/construct-core crates/construct-store
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(core): add RunStatus::Running for auto-apply pipelines

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

# PHASE 2 — Tag (auto-apply)

## Task 2.1: Vault scan — folders + existing tags

**Files:**
- Create: `crates/construct-obsidian/src/vault.rs`
- Modify: `crates/construct-obsidian/src/lib.rs`

- [ ] **Step 1: Write failing tests** — `vault.rs`:
```rust
use crate::frontmatter::Note;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// List relative folder paths under `root`, skipping dotfolders and `exclude`.
pub fn list_folders(root: &Path, exclude: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    walk_dirs(root, root, exclude, &mut out);
    out.sort();
    out
}

fn walk_dirs(root: &Path, dir: &Path, exclude: &[String], out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with('.') || exclude.iter().any(|e| e == name) {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().to_string());
        }
        walk_dirs(root, &path, exclude, out);
    }
}

/// Gather the set of tags already used across the vault (frontmatter + inline).
pub fn existing_tags(root: &Path) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    collect_tags(root, &mut set);
    set.into_iter().collect()
}

fn collect_tags(dir: &Path, set: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !name.starts_with('.') {
                collect_tags(&path, set);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let note = Note::parse(&text);
                for t in note.tags() {
                    set.insert(t);
                }
                if let Some(serde_yaml::Value::Sequence(seq)) =
                    note.frontmatter.get(serde_yaml::Value::from("tags"))
                {
                    for v in seq {
                        if let Some(s) = v.as_str() {
                            set.insert(s.to_string());
                        }
                    }
                }
            }
        }
    }
}

#[allow(dead_code)]
fn _types(_p: PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    #[test]
    fn lists_folders_skipping_dot_and_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Projects/Active")).unwrap();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::create_dir_all(root.join("Archive")).unwrap();
        let folders = list_folders(root, &["Archive".to_string()]);
        assert!(folders.contains(&"Projects".to_string()));
        assert!(folders.contains(&"Projects/Active".to_string()));
        assert!(!folders.iter().any(|f| f.contains(".obsidian")));
        assert!(!folders.contains(&"Archive".to_string()));
    }

    #[test]
    fn gathers_existing_tags() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("a.md"), "---\ntags:\n- rust\n- cli\n---\nbody #project");
        write(&root.join("sub/b.md"), "body #rust");
        let tags = existing_tags(root);
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"cli".to_string()));
        assert!(tags.contains(&"project".to_string()));
    }
}
```
Add `pub mod vault;` to `crates/construct-obsidian/src/lib.rs`.

- [ ] **Step 2: Run, expect PASS** (after compile)
Run: `cargo test -p construct-obsidian vault`
Expected: 2 tests pass.

- [ ] **Step 3: Commit**
```bash
git add crates/construct-obsidian
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(obsidian): vault scan for folders and existing tags

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2.2: Tag gate (normalize, cap, dedupe) + frontmatter merge

**Files:**
- Modify: `crates/construct-engine/src/gate.rs`, `crates/construct-obsidian/src/frontmatter.rs`

- [ ] **Step 1: Write failing frontmatter test** — in `frontmatter.rs` tests:
```rust
    #[test]
    fn merge_tags_unions_and_dedupes() {
        let mut note = Note::parse("---\ntags:\n- rust\n---\nbody");
        note.merge_tags(&["rust".into(), "cli".into()]);
        let out = note.to_string();
        let back = Note::parse(&out);
        let tags = back.frontmatter.get(serde_yaml::Value::from("tags")).unwrap();
        let seq = tags.as_sequence().unwrap();
        let vals: Vec<&str> = seq.iter().filter_map(|v| v.as_str()).collect();
        assert!(vals.contains(&"rust"));
        assert!(vals.contains(&"cli"));
        assert_eq!(vals.iter().filter(|t| **t == "rust").count(), 1); // no dup
    }
```

- [ ] **Step 2: Run, expect FAIL**
Run: `cargo test -p construct-obsidian merge_tags`

- [ ] **Step 3: Implement `merge_tags`** — add to `impl Note` in `frontmatter.rs`:
```rust
    /// Merge tags into the frontmatter `tags:` sequence (union, no duplicates,
    /// preserving existing). Creates the key if absent.
    pub fn merge_tags(&mut self, new_tags: &[String]) {
        use serde_yaml::Value;
        let key = Value::from("tags");
        let mut existing: Vec<String> = match self.frontmatter.get(&key) {
            Some(Value::Sequence(seq)) => seq
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            _ => Vec::new(),
        };
        for t in new_tags {
            if !existing.iter().any(|e| e == t) {
                existing.push(t.clone());
            }
        }
        let seq: Vec<Value> = existing.into_iter().map(Value::from).collect();
        self.frontmatter.insert(key, Value::Sequence(seq));
    }
```

- [ ] **Step 4: Run, expect PASS**
Run: `cargo test -p construct-obsidian merge_tags`

- [ ] **Step 5: Write failing gate test** — in `gate.rs` tests:
```rust
    #[test]
    fn tag_gate_normalizes_caps_dedupes() {
        let raw = r#"{"tags":["#Rust"," Web Dev ","rust","a","b","c","d","e","f","g"]}"#;
        let tags = validate_tags(raw, 8).unwrap();
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"web-dev".to_string()));
        assert_eq!(tags.iter().filter(|t| **t == "rust").count(), 1);
        assert!(tags.len() <= 8);
        assert!(matches!(validate_tags("nope", 8), Err(GateError::NotJson(_))));
        assert!(matches!(validate_tags(r#"{"tags":[]}"#, 8), Err(GateError::Invalid(_))));
    }
```

- [ ] **Step 6: Run, expect FAIL**
Run: `cargo test -p construct-engine tag_gate`

- [ ] **Step 7: Implement `validate_tags`** in `gate.rs`:
```rust
use crate::actions::TagsOut;

/// Normalize a single tag: lowercase, strip leading '#', spaces→'-', trim.
fn normalize_tag(t: &str) -> String {
    t.trim()
        .trim_start_matches('#')
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

/// Validate a tag action's output: valid JSON, normalize, dedupe, cap.
pub fn validate_tags(raw: &str, max_tags: usize) -> Result<Vec<String>, GateError> {
    let slice = extract_json(raw).ok_or_else(|| GateError::NotJson("no JSON object found".into()))?;
    let out: TagsOut =
        serde_json::from_str(slice).map_err(|e| GateError::NotJson(e.to_string()))?;
    let mut seen = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for t in out.tags {
        let n = normalize_tag(&t);
        if n.is_empty() || !seen.insert(n.clone()) {
            continue;
        }
        result.push(n);
        if result.len() >= max_tags {
            break;
        }
    }
    if result.is_empty() {
        return Err(GateError::Invalid("no usable tags".into()));
    }
    Ok(result)
}
```

- [ ] **Step 8: Run, expect PASS**
Run: `cargo test -p construct-engine tag_gate`

- [ ] **Step 9: Commit**
```bash
git add crates/construct-engine crates/construct-obsidian
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat: tag gate (normalize/cap/dedupe) + frontmatter merge_tags

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2.3: Tag transform + orchestrator wiring

**Files:**
- Create: `crates/construct-engine/src/pipelines/tag.rs`
- Modify: `crates/construct-engine/src/pipelines/mod.rs`, `crates/construct-engine/src/orchestrator.rs`

- [ ] **Step 1: Write failing transform test** — `pipelines/tag.rs`:
```rust
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
        let out = apply_tags(&text, &["new".into(), "old".into()], Some("theconstruct/done"));
        let note = Note::parse(&out);
        assert_eq!(note.get_str(STATUS_KEY).as_deref(), Some("done"));
        let seq = note
            .frontmatter
            .get(serde_yaml::Value::from("tags"))
            .unwrap()
            .as_sequence()
            .unwrap();
        let vals: Vec<&str> = seq.iter().filter_map(|v| v.as_str()).collect();
        assert!(vals.contains(&"new"));
        assert!(vals.contains(&"old"));
        assert_eq!(vals.iter().filter(|t| **t == "old").count(), 1);
    }
}
```
Add `pub mod tag;` to `pipelines/mod.rs`.

- [ ] **Step 2: Run, expect PASS**
Run: `cargo test -p construct-engine pipelines::tag`

- [ ] **Step 3: Add `vault_path` + `actions` to Orchestrator and wire `run_tag`.**

Add fields to `Orchestrator` (after `done_tag`):
```rust
    pub vault_path: std::path::PathBuf,
    pub max_tags: usize,
    pub exclude_dirs: Vec<String>,
```
Update the test `orch(...)` helper and the watch-loop constructor (Task 0.5 site) to set these:
```rust
        vault_path: dir.path().to_path_buf(),  // in tests
        max_tags: 8,
        exclude_dirs: vec![],
```
(In `watch_loop.rs`: `vault_path: vault.clone().into()`, `max_tags: cfg.actions.tag.max_tags`, `exclude_dirs: cfg.actions.organize.exclude_dirs.clone()`.)

Add the method:
```rust
    async fn run_tag(&self, run_id: &RunId, path: &Path, original: &str) -> anyhow::Result<()> {
        self.store.update_status(run_id, RunStatus::Running, None).await?;
        let note = Note::parse(original);
        let existing = construct_obsidian::vault::existing_tags(&self.vault_path);
        let user_prompt = format!(
            "Choose tags for this note. PREFER reusing these existing vault tags when they fit: {}. \
             Return STRICT JSON only: {{\"tags\": [string]}}.\n\nNOTE:\n{}",
            existing.join(", "),
            note.body
        );
        let messages = vec![
            ChatMessage::system(&self.system_prompt),
            ChatMessage::user(user_prompt),
        ];
        let out = match run_loop(
            self.provider.as_ref(),
            &self.tools,
            messages,
            &LoopConfig { model: self.model.clone(), max_iterations: self.max_iterations },
        )
        .await
        {
            Ok(r) => r,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let tags = match crate::gate::validate_tags(&out.content, self.max_tags) {
            Ok(t) => t,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let current = std::fs::read_to_string(path)?;
        std::fs::write(
            path,
            crate::pipelines::tag::apply_tags(&current, &tags, self.done_tag.as_deref()),
        )?;
        self.store.update_status(run_id, RunStatus::Done, None).await?;
        self.store
            .append_event(run_id, "tag", "done", serde_json::json!({"count": tags.len()}))
            .await?;
        Ok(())
    }
```
Wire the arm: `PipelineKind::Tag => self.run_tag(&run_id, path, &original).await,`.

- [ ] **Step 4: Write failing end-to-end test** — orchestrator tests:
```rust
    fn tags_model() -> ScriptedModel {
        ScriptedModel::new(vec![ChatResponse {
            message: ChatMessage::assistant(r#"{"tags":["rust","cli"]}"#),
        }])
    }

    #[tokio::test]
    async fn tag_auto_applies() {
        let dir = tempfile::tempdir().unwrap();
        let note_path = dir.path().join("n.md");
        std::fs::write(&note_path, "body about rust #theconstruct/tag").unwrap();
        let mut o = orch(Arc::new(tags_model())).await;
        o.pipeline = crate::pipelines::PipelineKind::Tag;
        o.vault_path = dir.path().to_path_buf();
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/tag".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: done"));
        assert!(after.contains("rust"));
        assert!(after.contains("cli"));
    }
```

- [ ] **Step 5: Run, expect PASS**
Run: `cargo test -p construct-engine`

- [ ] **Step 6: Commit**
```bash
git add crates/construct-engine crates/construct-cli
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(engine): wire tag pipeline end-to-end (prefers existing vault tags)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

# PHASE 3 — Organize (review-gated move)

## Task 3.1: Organize gate (destination must be a real folder)

**Files:**
- Modify: `crates/construct-engine/src/gate.rs`

- [ ] **Step 1: Write failing tests** — `gate.rs` tests:
```rust
    #[test]
    fn organize_gate_requires_known_destination() {
        let folders = vec!["Projects".to_string(), "Archive".to_string()];
        let ok = r#"{"destination":"Projects","reason":"active work"}"#;
        let o = validate_organize(ok, &folders).unwrap();
        assert_eq!(o.destination, "Projects");

        let bad = r#"{"destination":"Nonexistent","reason":"x"}"#;
        assert!(matches!(validate_organize(bad, &folders), Err(GateError::Invalid(_))));
        assert!(matches!(validate_organize("nope", &folders), Err(GateError::NotJson(_))));
    }
```

- [ ] **Step 2: Run, expect FAIL**
Run: `cargo test -p construct-engine organize_gate`

- [ ] **Step 3: Implement** in `gate.rs`:
```rust
use crate::actions::OrganizeOut;

/// Validate an organize action: valid JSON; destination must be one of `folders`.
pub fn validate_organize(raw: &str, folders: &[String]) -> Result<OrganizeOut, GateError> {
    let slice = extract_json(raw).ok_or_else(|| GateError::NotJson("no JSON object found".into()))?;
    let out: OrganizeOut =
        serde_json::from_str(slice).map_err(|e| GateError::NotJson(e.to_string()))?;
    let dest = out.destination.trim().trim_matches('/');
    if dest.is_empty() || !folders.iter().any(|f| f == dest) {
        return Err(GateError::Invalid(format!(
            "destination '{}' is not an existing vault folder",
            out.destination
        )));
    }
    Ok(OrganizeOut {
        destination: dest.to_string(),
        reason: out.reason,
    })
}
```

- [ ] **Step 4: Run, expect PASS**
Run: `cargo test -p construct-engine organize_gate`

- [ ] **Step 5: Commit**
```bash
git add crates/construct-engine
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(engine): organize gate (destination must be a real vault folder)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3.2: Organize transforms (propose / accept-move / reject)

**Files:**
- Create: `crates/construct-engine/src/pipelines/organize.rs`
- Modify: `crates/construct-engine/src/pipelines/mod.rs`

- [ ] **Step 1: Write failing tests** — `pipelines/organize.rs`:
```rust
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
```
Add `pub mod organize;` to `pipelines/mod.rs`.

- [ ] **Step 2: Run, expect PASS**
Run: `cargo test -p construct-engine pipelines::organize`

- [ ] **Step 3: Commit**
```bash
git add crates/construct-engine
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(engine): organize transforms (propose/accept/reject, pure)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3.3: Organize orchestration (propose → review → move on accept)

**Files:**
- Modify: `crates/construct-engine/src/orchestrator.rs`

- [ ] **Step 1: Write failing end-to-end test** — orchestrator tests:
```rust
    fn organize_model() -> ScriptedModel {
        ScriptedModel::new(vec![ChatResponse {
            message: ChatMessage::assistant(r#"{"destination":"Projects","reason":"active"}"#),
        }])
    }

    #[tokio::test]
    async fn organize_proposes_then_moves_on_accept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Projects")).unwrap();
        let note_path = dir.path().join("n.md");
        std::fs::write(&note_path, "body #theconstruct/organize").unwrap();
        let mut o = orch(Arc::new(organize_model())).await;
        o.pipeline = crate::pipelines::PipelineKind::Organize;
        o.vault_path = dir.path().to_path_buf();

        // propose → review (file not moved yet)
        o.handle(VaultEvent::NoteTagged {
            path: note_path.clone(),
            tag: "theconstruct/organize".into(),
        })
        .await
        .unwrap();
        let after = std::fs::read_to_string(&note_path).unwrap();
        assert!(after.contains("construct_status: review"));
        assert!(after.contains("construct_proposed_move: Projects"));
        assert!(note_path.exists());

        // accept → file moved into Projects/
        let accepted = after.replace("construct_status: review", "construct_status: accepted");
        std::fs::write(&note_path, &accepted).unwrap();
        o.handle(VaultEvent::StatusChanged {
            path: note_path.clone(),
            status: "accepted".into(),
        })
        .await
        .unwrap();
        assert!(!note_path.exists());
        let moved = dir.path().join("Projects/n.md");
        assert!(moved.exists());
        assert!(std::fs::read_to_string(&moved).unwrap().contains("construct_status: done"));
    }
```

- [ ] **Step 2: Run, expect FAIL**
Run: `cargo test -p construct-engine organize_proposes`

- [ ] **Step 3: Implement `run_organize` + accept handling.**

Add the propose method:
```rust
    async fn run_organize(&self, run_id: &RunId, path: &Path, original: &str) -> anyhow::Result<()> {
        self.store.update_status(run_id, RunStatus::Running, None).await?;
        let note = Note::parse(original);
        let folders = construct_obsidian::vault::list_folders(&self.vault_path, &self.exclude_dirs);
        let user_prompt = format!(
            "Pick the single best destination folder for this note from this list ONLY: {}. \
             Return STRICT JSON only: {{\"destination\": string, \"reason\": string}}.\n\nNOTE:\n{}",
            folders.join(", "),
            note.body
        );
        let messages = vec![
            ChatMessage::system(&self.system_prompt),
            ChatMessage::user(user_prompt),
        ];
        let out = match run_loop(
            self.provider.as_ref(),
            &self.tools,
            messages,
            &LoopConfig { model: self.model.clone(), max_iterations: self.max_iterations },
        )
        .await
        {
            Ok(r) => r,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let proposal = match crate::gate::validate_organize(&out.content, &folders) {
            Ok(p) => p,
            Err(e) => return self.fail(run_id, path, &e.to_string()).await,
        };
        let current = std::fs::read_to_string(path)?;
        std::fs::write(
            path,
            crate::pipelines::organize::apply_propose(&current, &proposal.destination, &proposal.reason),
        )?;
        self.store.update_status(run_id, RunStatus::Review, None).await?;
        self.store
            .append_event(run_id, "organize", "review", serde_json::json!({"destination": proposal.destination}))
            .await?;
        Ok(())
    }
```
Wire the arm: `PipelineKind::Organize => self.run_organize(&run_id, path, &original).await,`.

Extend `handle_decision` so accept performs the move for organize runs. Replace the `"accepted"` arm to branch on pipeline:
```rust
            "accepted" => {
                if self.pipeline == crate::pipelines::PipelineKind::Organize {
                    let dest = crate::pipelines::organize::proposed_destination(&current)
                        .ok_or_else(|| anyhow::anyhow!("no proposed_move on note"))?;
                    let updated = crate::pipelines::organize::apply_accept(&current, &note_path);
                    // compute target path with collision handling
                    let file_name = path.file_name().unwrap().to_string_lossy().to_string();
                    let mut target = self.vault_path.join(&dest).join(&file_name);
                    let mut n = 1;
                    while target.exists() {
                        let stem = path.file_stem().unwrap().to_string_lossy();
                        target = self.vault_path.join(&dest).join(format!("{stem} ({n}).md"));
                        n += 1;
                    }
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(path, &updated)?;
                    std::fs::rename(path, &target)?;
                    self.store.update_status(&run.id, RunStatus::Done, None).await?;
                    self.store
                        .append_event(&run.id, "finalize", "moved", serde_json::json!({"to": target.to_string_lossy()}))
                        .await?;
                } else {
                    std::fs::write(path, apply_accept(&current, self.done_tag.as_deref()))?;
                    self.store.update_status(&run.id, RunStatus::Done, None).await?;
                    self.store
                        .append_event(&run.id, "finalize", "done", serde_json::json!({}))
                        .await?;
                }
            }
```
And the `"rejected"` arm for organize uses `organize::apply_reject` when pipeline is Organize, else research `apply_reject`.

NOTE: `handle_decision` currently imports research `apply_accept`/`apply_reject`. Keep those imports; add the organize calls inline as shown.

- [ ] **Step 4: Run, expect PASS**
Run: `cargo test -p construct-engine`

- [ ] **Step 5: Commit**
```bash
git add crates/construct-engine
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(engine): organize pipeline — propose, review, move on accept

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

# PHASE 4 — Serialization, config sample, finalize

## Task 4.1: Per-note serialization (queue, don't skip)

**Files:**
- Modify: `crates/construct-cli/src/tui/watch_loop.rs`

Goal: ensure two actions on the same note never run concurrently. Use a per-note
`tokio::sync::Mutex` keyed by canonical path; the spawned handler acquires the
lock for that note before running.

- [ ] **Step 1: Implement** — add a shared lock map before the event loop:
```rust
    use std::collections::HashMap as Map2;
    use tokio::sync::Mutex as AsyncMutex;
    let note_locks: Arc<std::sync::Mutex<Map2<String, Arc<AsyncMutex<()>>>>> =
        Arc::new(std::sync::Mutex::new(Map2::new()));
```
In the `NoteTagged` spawn, acquire the per-note lock first:
```rust
            VaultEvent::NoteTagged { path, .. } => {
                if let Some(o) = orchestrators.get(&tag) {
                    let o = o.clone();
                    let key = path.to_string_lossy().to_string();
                    let lock = {
                        let mut map = note_locks.lock().unwrap();
                        map.entry(key).or_insert_with(|| Arc::new(AsyncMutex::new(()))).clone()
                    };
                    tokio::spawn(async move {
                        let _guard = lock.lock().await; // serialize per note
                        if let Err(e) = o.handle(event).await {
                            tracing::error!("handler error: {e}");
                        }
                    });
                }
            }
```
This makes a second action on the same note wait for the first to finish rather
than running concurrently. (Cross-note actions still run in parallel.)

- [ ] **Step 2: Run, expect PASS** (compiles; existing CLI tests pass)
Run: `cargo build && cargo test -p construct-cli`

- [ ] **Step 3: Commit**
```bash
git add crates/construct-cli
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(cli): serialize actions per note via per-path async lock

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4.2: Sample config + Librarian prompt

**Files:**
- Modify: `crates/construct-cli/src/commands.rs`
- Create: `prompts/librarian.md`

- [ ] **Step 1: Update `SAMPLE_CONFIG`** in `commands.rs` — append the Librarian agent and three rules, and `[actions.*]`:
```toml
[[agents]]
name = "Librarian"
domain = "notes"
provider = "ollama"
model = "qwen3:4b-instruct-2507-q8_0"
base_url = "http://192.168.1.33:11434"
tools = []
system_prompt_file = "prompts/librarian.md"

[[rules]]
match_tag = "theconstruct/summarize"
agent = "Librarian"
pipeline = "summarize"

[[rules]]
match_tag = "theconstruct/tag"
agent = "Librarian"
pipeline = "tag"

[[rules]]
match_tag = "theconstruct/organize"
agent = "Librarian"
pipeline = "organize"

[actions.tag]
max_tags = 8

[actions.organize]
exclude_dirs = [".obsidian", ".trash"]
```
Update the `sample_config_is_valid` test to assert 2 agents and 4 rules and that `validate()` passes.

- [ ] **Step 2: Create `prompts/librarian.md`**
```markdown
You are Librarian, a careful note-organizing assistant for The Construct.

You will be asked to perform ONE of: summarize, tag, or organize a note.
Always respond with STRICT JSON only — no prose, no markdown fences, no commentary.

- summarize → {"tldr": "2-4 sentence summary", "action_items": ["..."]}
- tag → {"tags": ["lowercase-hyphenated", "..."]}  (prefer the existing tags you are shown; reuse before inventing)
- organize → {"destination": "<one of the folders you are shown>", "reason": "short why"}

Never output a folder or field that was not requested. Output JSON and nothing else.
```

- [ ] **Step 3: Run, expect PASS**
Run: `cargo test -p construct-cli`

- [ ] **Step 4: Commit**
```bash
git add crates/construct-cli prompts/librarian.md
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "feat(cli): sample config gains Librarian agent + action rules; add prompt

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4.3: Workspace verification + README + docs

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update README** — add a "Note actions" section documenting the three tags, auto-apply vs review for organize, and the `construct_status` flow. Keep it short; mirror the research section's style.

- [ ] **Step 2: Full sweep**
Run:
```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```
All tests pass; clippy clean (fix minimally, behavior-preserving); fmt clean (`cargo fmt --all` then re-check).

- [ ] **Step 3: Commit**
```bash
git add -A
git -c user.name="Matt" -c user.email="matt@matthewlittlehale.com" commit -m "docs: document note actions; clippy + fmt clean for Slice 2

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage:**
- Phase 0 dispatch refactor (spec §3) → Tasks 0.1–0.5 ✓
- Summarize, auto-apply, top managed block (spec §4.1) → Tasks 1.1–1.4 ✓
- Tag, prefer-existing, normalize/cap/merge (spec §4.2) → Tasks 2.1–2.3 ✓
- Organize, live folder scan, guard, review→move, moved_from (spec §4.3) → Tasks 3.1–3.3 ✓
- One-tag-per-action triggering (spec §5) → Task 0.5 routing + 4.2 rules ✓
- Serialize per note (spec §5) → Task 4.1 ✓
- Config additions + validation (spec §6) → Tasks 0.1, 4.2 ✓
- Component/file map (spec §7) → matches File Structure ✓
- Error handling via fail() (spec §8) → reused in every run_* method ✓
- Testing strategy (spec §9) → per-task tests + 4.3 sweep ✓
- Appendices A/B are documentation-only → no tasks (correct; deferred) ✓

**2. Placeholder scan:** No TBD/TODO. Every code step shows full code. The only
forward references ("fields added in later tasks") are explicit and the adding
task updates the call site in the same task (Tasks 1.4, 2.3, 3.3 update the
struct + both call sites). Ordering caveat called out: do Task 1.4 before 1.3
compiles.

**3. Type consistency:** `PipelineKind` (Research/Summarize/Tag/Organize),
`RunStatus::Running`, gate fns (`validate_summary`, `validate_tags(raw,max)`,
`validate_organize(raw,&folders)`), action types (`SummaryOut`, `TagsOut`,
`OrganizeOut`), block fns (`upsert_named`, `upsert_named_at_top`, `remove_named`),
`Note::merge_tags`, organize keys (`MOVE_KEY`/`REASON_KEY`/`MOVED_FROM_KEY`), and
the new Orchestrator fields (`pipeline`, `vault_path`, `max_tags`, `exclude_dirs`)
are used consistently across tasks. `apply_claim`/`STATUS_KEY`/`RUN_KEY` are
shared from `pipelines::mod`.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-30-the-construct-slice-2-note-actions.md`. Per your instruction, implementation is **on hold** — nothing here will be built until you say go.

When you're ready, two execution options:
1. **Subagent-Driven (recommended)** — fresh subagent per task, review between tasks (what we used for Slice 1).
2. **Inline Execution** — execute in-session with checkpoints.
