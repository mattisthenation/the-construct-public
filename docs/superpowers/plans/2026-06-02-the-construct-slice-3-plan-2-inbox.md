# Slice 3 — Plan 2: Inbox Auto-Processing

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a top-level note in the configured `Inbox/` folder goes idle (≥ `idle_minutes` by file mtime, default 30) and has no `construct_status`, automatically: enrich it from its first 5 URLs, summarize it, tag it, and either auto-move it to an existing folder (status `done`) or leave it in Inbox with a recommended destination written at the top (frontmatter `construct_status: review`). Each processed note is logged in `Inbox/_index.md`.

**Architecture:** Builds on Plan 1's `IdleTrigger` core, `Clock`, loop-guard, `TriggerEvent`, and `PipelineKind::Inbox`. Adds: pure helpers in a new `pipelines/inbox.rs`, a loose destination gate, a `read_named` block reader, a testable `scan_inbox` directory scanner, the `run_inbox` orchestrator method (replacing Plan 1's stub arm), and the watch-loop wiring (build an Inbox orchestrator from `[inbox]` config + spawn the idle poller + route `IdleNote`).

**Key behavioral decisions (read before coding):**
- **No-reprocess guarantee:** `scan_inbox` skips any note whose frontmatter has a `construct_status` (via `should_process_inbox_note` from Plan 1), plus `_index` and managed files (via `is_excluded`). `handle_tagged`'s claim stamps `construct_status: queued` immediately, so the next poll tick won't re-emit a note mid-processing.
- **Re-run story:** the recommendation (unmoved) outcome sets the note's **frontmatter** `construct_status: review` BUT marks the **store run** `Done`. Rationale: an Inbox recommendation has no automation follow-up (unlike organize's Review, which awaits accept/reject). Making the store run terminal means that when the user later clears the note's frontmatter `construct_status` to force a re-run, `handle_tagged`'s `run_for_note` idempotency guard (which blocks on non-terminal runs) does NOT block it. The frontmatter `review` is purely human-facing advice.
- **Looser-than-organize move gate:** auto-move ONLY if the model's destination exactly matches an existing scanned folder. Otherwise the destination is a recommendation (may name a not-yet-existing folder); Construct never creates a folder and never auto-moves into a non-existing one. This uses `validate_destination` (shape-only, no folder-membership check) + a separate "is this an existing folder?" branch — NOT `validate_organize` (which fails the gate on unknown folders).

**Tech Stack:** Rust workspace, tokio, chrono, sqlx. Reuses `apply_summary`/`render_summary`, `merge_tags`, `validate_summary`/`validate_tags`, managed blocks, the move-with-collision logic, `WebFetch`, `run_loop`. Commit author `Matt <matt@matthewlittlehale.com>` with trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

**Conventions:** TDD per task. Keep `cargo test`/`clippy --all-targets -- -D warnings`/`fmt --all -- --check` green at every commit.

---

## File Structure

- `crates/construct-obsidian/src/block.rs` — **MODIFY**: add `read_named`.
- `crates/construct-engine/src/gate.rs` — **MODIFY**: add `validate_destination`.
- `crates/construct-engine/src/pipelines/inbox.rs` — **NEW**: `extract_urls`, `apply_recommendation`, `update_index`.
- `crates/construct-engine/src/pipelines/mod.rs` — **MODIFY**: `pub mod inbox;`.
- `crates/construct-config/src/lib.rs` — **MODIFY**: add `agent: Option<String>` to `InboxCfg` + validation.
- `crates/construct-engine/src/orchestrator.rs` — **MODIFY**: `run_inbox` method, `handle_idle` entry, `collision_free_target` helper, replace `Inbox` stub arm.
- `crates/construct-engine/src/triggers/idle.rs` — **MODIFY**: add `scan_inbox`.
- `crates/construct-engine/Cargo.toml` — **MODIFY**: add `filetime` dev-dependency.
- `crates/construct-cli/src/tui/watch_loop.rs` — **MODIFY**: build Inbox orchestrator + spawn idle poller + route `IdleNote`.

---

### Task 1: `read_named` block reader

**Files:**
- Modify: `crates/construct-obsidian/src/block.rs`

To maintain the `_index` log idempotently we must read the current managed-block content, append, and re-upsert. `block.rs` has `upsert_named`/`remove_named` but no reader.

- [ ] **Step 1: Write the failing test** — add to the `tests` module in `block.rs`:

```rust
    #[test]
    fn read_named_returns_inner_content() {
        let b = upsert_named("body", "inbox-log", "line one\nline two");
        assert_eq!(read_named(&b, "inbox-log").as_deref(), Some("line one\nline two"));
        assert_eq!(read_named(&b, "absent"), None);
        assert_eq!(read_named("no blocks here", "inbox-log"), None);
    }
```

- [ ] **Step 2: Run to confirm fail**

Run: `cargo test -p construct-obsidian read_named_returns_inner_content`
Expected: FAIL (function not found).

- [ ] **Step 3: Implement** — add to `block.rs` (after `remove_named`):

```rust
/// Read the inner content of a named managed block, if present.
/// Returns the text between the start and end markers (trimming the single
/// newline that `upsert_named` inserts on each side).
pub fn read_named(body: &str, name: &str) -> Option<String> {
    let (start, end) = markers(name);
    let s = body.find(&start)? + start.len();
    let e = body.find(&end)?;
    if e < s {
        return None;
    }
    Some(body[s..e].trim_matches('\n').to_string())
}
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p construct-obsidian read_named`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/construct-obsidian/src/block.rs
git commit -m "feat(obsidian): read_named block reader" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```
(Use `git -c user.name=Matt -c user.email=matt@matthewlittlehale.com commit ...`.)

---

### Task 2: Loose destination gate `validate_destination`

**Files:**
- Modify: `crates/construct-engine/src/gate.rs`

The Inbox move decision needs the model's proposed destination + reason WITHOUT rejecting unknown folders (unknown = a recommendation). This is shape-validation only: valid JSON, non-empty destination. The existing `validate_organize` (folder-membership check) is unchanged and still used by the organize pipeline.

- [ ] **Step 1: Write the failing test** — add to the `tests` module in `gate.rs`:

```rust
    #[test]
    fn validate_destination_accepts_any_nonempty_dest() {
        // No folder list: any non-empty destination is accepted (it may be a new-folder suggestion).
        let ok = r#"{"destination":"Reading/Articles","reason":"it's an article"}"#;
        let o = validate_destination(ok).unwrap();
        assert_eq!(o.destination, "Reading/Articles");
        assert_eq!(o.reason, "it's an article");

        // Trims surrounding slashes/space like validate_organize does.
        let trimmed = r#"{"destination":" /Projects/ ","reason":"x"}"#;
        assert_eq!(validate_destination(trimmed).unwrap().destination, "Projects");

        // Empty destination is rejected.
        assert!(matches!(
            validate_destination(r#"{"destination":"  ","reason":"x"}"#),
            Err(GateError::Invalid(_))
        ));
        // Non-JSON rejected.
        assert!(matches!(validate_destination("nope"), Err(GateError::NotJson(_))));
    }
```

- [ ] **Step 2: Run to confirm fail**

Run: `cargo test -p construct-engine validate_destination_accepts_any_nonempty_dest`
Expected: FAIL (function not found).

- [ ] **Step 3: Implement** — add to `gate.rs` (after `validate_organize`):

```rust
/// Validate an Inbox move suggestion: valid JSON, non-empty destination.
/// Unlike `validate_organize`, the destination is NOT required to be an existing
/// folder — an unknown destination is a recommendation for the human, not an error.
/// The caller decides whether to auto-move (destination is an existing folder) or
/// merely recommend (destination does not yet exist).
pub fn validate_destination(raw: &str) -> Result<OrganizeOut, GateError> {
    let slice =
        extract_json(raw).ok_or_else(|| GateError::NotJson("no JSON object found".into()))?;
    let out: OrganizeOut =
        serde_json::from_str(slice).map_err(|e| GateError::NotJson(e.to_string()))?;
    let dest = out.destination.trim().trim_matches('/').trim();
    if dest.is_empty() {
        return Err(GateError::Invalid("destination is empty".into()));
    }
    Ok(OrganizeOut {
        destination: dest.to_string(),
        reason: out.reason,
    })
}
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p construct-engine validate_destination`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/construct-engine/src/gate.rs
git commit -m "feat(engine): validate_destination loose gate for inbox moves" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: `pipelines/inbox.rs` — `extract_urls`

**Files:**
- Create: `crates/construct-engine/src/pipelines/inbox.rs`
- Modify: `crates/construct-engine/src/pipelines/mod.rs`

Pure URL extraction (no regex dependency): scan for `http://` / `https://` and read until whitespace or a closing markdown/paren delimiter. Cap at `max`. Preserve document order, dedupe.

- [ ] **Step 1: Write the failing test** — `crates/construct-engine/src/pipelines/inbox.rs`:

```rust
//! Inbox pipeline pure helpers (URL extraction, recommendation block, _index log).

/// Extract up to `max` distinct URLs (http/https) from `text`, in document order.
/// Trailing punctuation and markdown/paren delimiters are stripped.
pub fn extract_urls(text: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let starts = rest.starts_with("http://") || rest.starts_with("https://");
        if starts {
            // Read until whitespace or a delimiter that cannot be part of a URL.
            let end = rest
                .find(|c: char| c.is_whitespace() || matches!(c, ')' | ']' | '>' | '"' | '\'' | '|'))
                .unwrap_or(rest.len());
            let raw = &rest[..end];
            // Strip trailing sentence punctuation.
            let url = raw.trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '!' | '?'));
            if !url.is_empty() && !out.iter().any(|u| u == url) {
                out.push(url.to_string());
                if out.len() >= max {
                    break;
                }
            }
            i += end.max(1);
        } else {
            // advance one UTF-8 char
            let step = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            i += step;
        }
    }
    let _ = bytes;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_zero_urls() {
        assert!(extract_urls("just some text, no links", 5).is_empty());
    }

    #[test]
    fn extracts_single_url_stripping_trailing_punctuation() {
        let urls = extract_urls("see https://example.com/page. cool", 5);
        assert_eq!(urls, vec!["https://example.com/page".to_string()]);
    }

    #[test]
    fn extracts_markdown_link_url() {
        let urls = extract_urls("[a](https://example.com/x) and more", 5);
        assert_eq!(urls, vec!["https://example.com/x".to_string()]);
    }

    #[test]
    fn caps_at_max_five_and_dedupes() {
        let text = "https://a.com https://b.com https://c.com https://a.com \
                    https://d.com https://e.com https://f.com https://g.com";
        let urls = extract_urls(text, 5);
        assert_eq!(urls.len(), 5);
        assert_eq!(urls[0], "https://a.com");
        // a.com appears once despite repeating
        assert_eq!(urls.iter().filter(|u| *u == "https://a.com").count(), 1);
        assert!(!urls.contains(&"https://f.com".to_string()));
    }

    #[test]
    fn ignores_non_http_schemes() {
        assert!(extract_urls("ftp://x.com mailto:a@b.com", 5).is_empty());
    }
}
```

> The `let _ = bytes;` line silences an unused-variable lint if you don't use `bytes`; remove the `bytes` binding entirely if clippy prefers (simplest: delete the `let bytes = ...` and the `let _ = bytes;` lines — they are not needed since indexing uses `&text[i..]`). Prefer deleting both; keep the function clean.

- [ ] **Step 2: Register the module** — in `crates/construct-engine/src/pipelines/mod.rs`, add near the other `pub mod` lines:

```rust
pub mod inbox;
```

- [ ] **Step 3: Run to confirm fail then pass**

Run: `cargo test -p construct-engine extract_urls`
Expected: PASS (5 tests). Run `cargo clippy -p construct-engine --all-targets -- -D warnings` to confirm no unused-variable warnings (delete the `bytes` lines if flagged).

- [ ] **Step 4: Commit**

```bash
git add crates/construct-engine/src/pipelines/inbox.rs crates/construct-engine/src/pipelines/mod.rs
git commit -m "feat(engine): inbox URL extraction (first-5, dedupe)" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: `pipelines/inbox.rs` — `apply_recommendation` + `update_index`

**Files:**
- Modify: `crates/construct-engine/src/pipelines/inbox.rs`

`apply_recommendation` writes a managed block at the TOP of the note recommending a destination and sets frontmatter `construct_status: review`. `update_index` maintains the `inbox-log` managed block in `Inbox/_index.md`: one line per note (keyed by note name), de-duped so re-processing updates in place.

- [ ] **Step 1: Write the failing tests** — add to the `tests` module in `inbox.rs`:

```rust
    use construct_obsidian::frontmatter::Note;

    #[test]
    fn apply_recommendation_adds_block_and_review_status() {
        let text = crate::pipelines::apply_claim("My note body", "r1");
        let out = apply_recommendation(&text, "Reading/Articles", "looks like an article");
        let note = Note::parse(&out);
        assert_eq!(
            note.get_str(crate::pipelines::STATUS_KEY).as_deref(),
            Some("review")
        );
        assert!(note
            .body
            .trim_start()
            .starts_with("<!-- construct:inbox-recommendation:start -->"));
        assert!(out.contains("Reading/Articles"));
        assert!(out.contains("looks like an article"));
        assert!(out.contains("My note body"));
    }

    #[test]
    fn apply_recommendation_rerun_replaces_single_block() {
        let text = crate::pipelines::apply_claim("body", "r1");
        let once = apply_recommendation(&text, "A", "r1");
        let twice = apply_recommendation(&once, "B", "r2");
        assert_eq!(twice.matches("construct:inbox-recommendation:start").count(), 1);
        assert!(twice.contains("B"));
    }

    #[test]
    fn update_index_appends_and_dedupes_by_note() {
        let idx = update_index("", "idea.md", "summarized, tagged, recommended→Reading");
        assert!(idx.contains("<!-- construct:inbox-log:start -->"));
        assert!(idx.contains("idea.md"));
        assert!(idx.contains("recommended→Reading"));

        // A second note adds a new line.
        let idx = update_index(&idx, "todo.md", "moved→Projects");
        assert!(idx.contains("idea.md"));
        assert!(idx.contains("todo.md"));

        // Re-processing idea.md replaces its line, not duplicates it.
        let idx = update_index(&idx, "idea.md", "moved→Archive");
        assert_eq!(idx.matches("idea.md").count(), 1);
        assert!(idx.contains("moved→Archive"));
        assert!(!idx.contains("recommended→Reading"));
        assert!(idx.contains("todo.md")); // other entries preserved
    }
```

- [ ] **Step 2: Implement** — add to `inbox.rs` (above the `#[cfg(test)]`):

```rust
use construct_obsidian::block::{read_named, upsert_named, upsert_named_at_top};
use construct_obsidian::frontmatter::Note;

const RECOMMENDATION_BLOCK: &str = "inbox-recommendation";
const INDEX_BLOCK: &str = "inbox-log";

/// Render + insert the recommendation block at the top of the note, set status=review.
/// The note stays in Inbox; this is human-facing advice (Construct does not move it).
pub fn apply_recommendation(text: &str, destination: &str, reason: &str) -> String {
    let mut note = Note::parse(text);
    let content = format!(
        "> [!tip] Suggested location: **{destination}**\n> {reason}\n> \
         _Move it there (or clear `construct_status` to re-run)._"
    );
    note.body = upsert_named_at_top(&note.body, RECOMMENDATION_BLOCK, &content);
    note.set_str(crate::pipelines::STATUS_KEY, "review");
    note.to_string()
}

/// Maintain the `inbox-log` managed block in `Inbox/_index.md`. One markdown bullet
/// per processed note, keyed by note file name so re-processing updates in place.
/// `index_text` is the current full text of `_index.md` ("" if it does not exist yet).
pub fn update_index(index_text: &str, note_name: &str, outcome: &str) -> String {
    let existing = read_named(index_text, INDEX_BLOCK).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| !l.trim().is_empty())
        // drop any prior line for this same note (dedupe / update-in-place)
        .filter(|l| !line_is_for_note(l, note_name))
        .map(|l| l.to_string())
        .collect();
    lines.push(format!("- `{note_name}` — {outcome}"));
    upsert_named(index_text, INDEX_BLOCK, &lines.join("\n"))
}

/// True if a log line refers to `note_name` (matches the `\`<name>\`` token we write).
fn line_is_for_note(line: &str, note_name: &str) -> bool {
    line.contains(&format!("`{note_name}`"))
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p construct-engine inbox`
Expected: PASS (extract_urls + the 3 new tests).

- [ ] **Step 4: Commit**

```bash
git add crates/construct-engine/src/pipelines/inbox.rs
git commit -m "feat(engine): inbox recommendation block + _index log helpers" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Config — `InboxCfg.agent`

**Files:**
- Modify: `crates/construct-config/src/lib.rs`

The Inbox orchestrator needs a model/provider/tools, which come from a configured `Agent`. Add an optional `agent` name to `[inbox]`. If absent, the watch loop falls back (Task 9) to a sensible default agent. If present, `validate()` requires it to be a defined agent.

- [ ] **Step 1: Write the failing tests** — add to the `tests` module in `lib.rs`:

```rust
    #[test]
    fn inbox_agent_defaults_to_none() {
        let toml = format!("{}\n[inbox]\n", sample());
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert!(cfg.inbox.unwrap().agent.is_none());
    }

    #[test]
    fn inbox_agent_must_exist_when_named() {
        // Scout IS defined in sample(); Ghost is not.
        let good = format!("{}\n[inbox]\nagent = \"Scout\"\n", sample());
        toml::from_str::<Config>(&good).unwrap().validate().unwrap();

        let bad = format!("{}\n[inbox]\nagent = \"Ghost\"\n", sample());
        let cfg: Config = toml::from_str(&bad).unwrap();
        assert!(matches!(cfg.validate(), Err(ConfigError::Validation(_))));
    }
```

- [ ] **Step 2: Add the field** — in `InboxCfg` (created in Plan 1):

```rust
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct InboxCfg {
    #[serde(default = "default_inbox_folder")]
    pub folder: String,
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u64,
    #[serde(default)]
    pub agent: Option<String>,
}
```

- [ ] **Step 3: Add validation** — in `Config::validate()`, inside the existing `if let Some(inbox) = &self.inbox {` block (add after the idle_minutes check):

```rust
            if let Some(agent) = &inbox.agent {
                if !self.agents.iter().any(|a| &a.name == agent) {
                    return Err(ConfigError::Validation(format!(
                        "inbox.agent references unknown agent '{agent}'"
                    )));
                }
            }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p construct-config`
Expected: PASS (existing Plan 1 config tests + 2 new). The Plan 1 test `parses_inbox_journal_schedule` still passes (it does not construct `InboxCfg` literally).

- [ ] **Step 5: Commit**

```bash
git add crates/construct-config/src/lib.rs
git commit -m "feat(config): optional inbox.agent + validation" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: Orchestrator — `run_inbox` + `handle_idle` + `collision_free_target`

**Files:**
- Modify: `crates/construct-engine/src/orchestrator.rs`

This is the integration heart. `run_inbox` runs the five steps in order, writing the note once at the end. It reuses `WebFetch` (via `self.tools["web_fetch"]`), `run_loop`, `validate_summary`/`validate_tags`/`validate_destination`, `render_summary`, `merge_tags`, managed blocks, and the move-with-collision logic (extracted into a shared `collision_free_target` helper also used by `handle_decision`).

**Behavior:**
1. status → Running. Re-read note from disk (claim already stamped queued).
2. **URL enrich:** `extract_urls(body, 5)`; for each, fetch via `web_fetch` tool (skip on fetch error); summarize the fetched text via `run_loop` + `validate_summary` (skip on error). Collect `(url, tldr)`. If any succeeded, upsert an `inbox-links` managed block listing them.
3. **Summarize note:** `run_loop` + `validate_summary` → `upsert_named_at_top(body, "summary", render_summary(&s))`. On gate failure → `fail()`.
4. **Tag note:** gather `existing_tags_excluding`, `run_loop` + `validate_tags` → `note.merge_tags(&tags)`. On gate failure → `fail()`.
5. **Move decision:** list folders; `run_loop` + `validate_destination`. If `destination` exactly matches an existing folder → set status=done, remove run id, write note, then move file with collision handling (store run → Done, event "moved"). Else → `apply_recommendation` (frontmatter review), write note in place, store run → **Done** (terminal; see plan header re re-run), event "recommended".
6. **`_index` update** (best-effort; failure is logged, not fatal): read/maintain `Inbox/_index.md` `inbox-log` block with the outcome line. The `_index` path = the note's parent dir + `_index.md`.

- [ ] **Step 1: Write the failing integration tests** — add to the `tests` module in `orchestrator.rs`. Reuse the existing test scaffolding (`ScriptedModel`, `EchoTool`, in-memory `SqliteStore`). Model the helper that builds an Inbox orchestrator on the patterns already in this test module (find an existing test that constructs an `Orchestrator{...}` and copy its field initialization, setting `pipeline: PipelineKind::Inbox`, `rule: "inbox".into()`, and a `tools` map containing `"web_fetch" => Arc::new(EchoTool::new("web_fetch", "<fetched page text>"))`).

```rust
    #[tokio::test]
    async fn inbox_recommends_when_destination_is_unknown_folder() {
        use crate::pipelines::PipelineKind;
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let inbox_dir = vault.join("Inbox");
        std::fs::create_dir_all(&inbox_dir).unwrap();
        // An existing folder so list_folders is non-empty but does NOT match the suggestion.
        std::fs::create_dir_all(vault.join("Projects")).unwrap();
        let note_path = inbox_dir.join("idea.md");
        std::fs::write(&note_path, "A thought. See https://example.com/x for more.").unwrap();

        // Scripted model responses, in the order run_inbox consumes them:
        //   1 per fetched URL summary, then note summary, then tags, then destination.
        let model = ScriptedModel::new(vec![
            chat_text(r#"{"tldr":"Link summary.","action_items":[]}"#), // URL #1 summary
            chat_text(r#"{"tldr":"A short thought.","action_items":["follow up"]}"#), // note summary
            chat_text(r#"{"tags":["idea","reading"]}"#), // tags
            chat_text(r#"{"destination":"Reading/Articles","reason":"it is an article"}"#), // move decision (unknown folder)
        ]);
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert(
            "web_fetch".into(),
            Arc::new(EchoTool::new("web_fetch", "fetched page text about something")),
        );
        let store: Arc<dyn Store> = Arc::new(SqliteStore::connect("sqlite::memory:").await.unwrap());
        let orch = Orchestrator {
            store: store.clone(),
            provider: Arc::new(model),
            tools,
            model: "m".into(),
            agent: "Librarian".into(),
            rule: "inbox".into(),
            pipeline: PipelineKind::Inbox,
            system_prompt: "You are the Librarian.".into(),
            max_iterations: 4,
            done_tag: None,
            vault_path: vault.to_path_buf(),
            max_tags: 8,
            exclude_dirs: vec![],
        };

        orch.handle_idle(&note_path).await.unwrap();

        let out = std::fs::read_to_string(&note_path).unwrap();
        let note = Note::parse(&out);
        // Stays in Inbox, frontmatter review, recommendation block present.
        assert!(note_path.exists());
        assert_eq!(note.get_str("construct_status").as_deref(), Some("review"));
        assert!(out.contains("construct:inbox-recommendation:start"));
        assert!(out.contains("Reading/Articles"));
        assert!(out.contains("construct:summary:start")); // summarized
        assert!(out.contains("idea")); // tagged (frontmatter tags)
        assert!(out.contains("construct:inbox-links:start")); // url enriched
        // _index written with the outcome.
        let idx = std::fs::read_to_string(inbox_dir.join("_index.md")).unwrap();
        assert!(idx.contains("idea.md"));
        // Store run is terminal (Done) so a future re-run isn't blocked.
        let run = store.run_for_note(&note_path.to_string_lossy()).await.unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Done);
    }

    #[tokio::test]
    async fn inbox_auto_moves_when_destination_is_existing_folder() {
        use crate::pipelines::PipelineKind;
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let inbox_dir = vault.join("Inbox");
        std::fs::create_dir_all(&inbox_dir).unwrap();
        std::fs::create_dir_all(vault.join("Projects")).unwrap();
        let note_path = inbox_dir.join("task.md");
        std::fs::write(&note_path, "No links here, just a task.").unwrap();

        // No URLs → no per-URL summary response. Order: note summary, tags, destination.
        let model = ScriptedModel::new(vec![
            chat_text(r#"{"tldr":"A task.","action_items":[]}"#),
            chat_text(r#"{"tags":["task"]}"#),
            chat_text(r#"{"destination":"Projects","reason":"it is a project task"}"#),
        ]);
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("web_fetch".into(), Arc::new(EchoTool::new("web_fetch", "x")));
        let store: Arc<dyn Store> = Arc::new(SqliteStore::connect("sqlite::memory:").await.unwrap());
        let orch = Orchestrator {
            store: store.clone(),
            provider: Arc::new(model),
            tools,
            model: "m".into(),
            agent: "Librarian".into(),
            rule: "inbox".into(),
            pipeline: PipelineKind::Inbox,
            system_prompt: "You are the Librarian.".into(),
            max_iterations: 4,
            done_tag: None,
            vault_path: vault.to_path_buf(),
            max_tags: 8,
            exclude_dirs: vec![],
        };

        orch.handle_idle(&note_path).await.unwrap();

        // Moved out of Inbox into Projects, status done.
        assert!(!note_path.exists());
        let moved = vault.join("Projects").join("task.md");
        assert!(moved.exists());
        let note = Note::parse(&std::fs::read_to_string(&moved).unwrap());
        assert_eq!(note.get_str("construct_status").as_deref(), Some("done"));
    }
```

> **Test helper:** the test module needs a `chat_text(s: &str) -> ChatResponse` constructor that builds a `ChatResponse` with `content = s` and no tool calls. Check whether the existing tests already build `ChatResponse` values (e.g. via a helper or inline struct literal); if a helper exists reuse it, otherwise add a small `fn chat_text(s: &str) -> ChatResponse` mirroring how the existing research/summarize tests construct scripted responses. Match the existing `ChatResponse` shape exactly.

- [ ] **Step 2: Run to confirm fail**

Run: `cargo test -p construct-engine inbox_recommends inbox_auto_moves`
Expected: FAIL (`handle_idle`/`run_inbox` not implemented; Inbox arm currently calls `fail`).

- [ ] **Step 3: Add `handle_idle`** — in `orchestrator.rs`, in the `impl Orchestrator` block, near `handle`:

```rust
    /// Entry point for the idle (Inbox) trigger. Reuses the same claim +
    /// idempotency path as a tag trigger, then dispatches the Inbox pipeline.
    pub async fn handle_idle(&self, path: &Path) -> anyhow::Result<()> {
        self.handle_tagged(path).await
    }
```

- [ ] **Step 4: Replace the Inbox stub arm** — in `handle_tagged`'s `match self.pipeline`, change the `PipelineKind::Inbox` arm from the Plan 1 stub to:

```rust
            PipelineKind::Inbox => self.run_inbox(&run_id, path, &original).await,
```

- [ ] **Step 5: Extract the move helper** — add this private method (and refactor `handle_decision`'s organize-accept move loop to call it, so both paths share one implementation):

```rust
    /// Compute a collision-free absolute target path for moving `path` into the
    /// vault-relative folder `dest`: `<vault>/<dest>/<name>.md`, appending
    /// ` (1)`, ` (2)`, … to the stem until the path is free.
    fn collision_free_target(&self, dest: &str, path: &Path) -> std::path::PathBuf {
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let mut target = self.vault_path.join(dest).join(&file_name);
        let mut n = 1;
        while target.exists() {
            let stem = path.file_stem().unwrap().to_string_lossy();
            target = self.vault_path.join(dest).join(format!("{stem} ({n}).md"));
            n += 1;
        }
        target
    }
```

In `handle_decision`'s organize-accept branch, replace the inline `let mut target = ...; while target.exists() { ... }` loop with:

```rust
                    let target = self.collision_free_target(&dest, path);
```

(Keep the subsequent `create_dir_all(parent)` + `rename` + `write` + store update exactly as before.)

- [ ] **Step 6: Implement `run_inbox`** — add this method to `impl Orchestrator`:

```rust
    /// Inbox pipeline: enrich from URLs → summarize → tag → move-or-recommend → log.
    /// Writes the note once at the end. Auto-moves only into an existing folder;
    /// otherwise leaves the note in Inbox with a recommended destination at the top.
    async fn run_inbox(&self, run_id: &RunId, path: &Path, _original: &str) -> anyhow::Result<()> {
        use crate::pipelines::inbox;
        self.store
            .update_status(run_id, RunStatus::Running, None)
            .await?;

        // Re-read post-claim content; build the note in memory, write once at the end.
        let current = std::fs::read_to_string(path)?;
        let mut note = Note::parse(&current);

        // --- Step 1: URL enrich (skip-on-fail) ---
        let urls = inbox::extract_urls(&note.body, 5);
        let mut link_lines: Vec<String> = Vec::new();
        if let Some(fetch) = self.tools.get("web_fetch") {
            for url in &urls {
                let fetched = match fetch.call(serde_json::json!({ "url": url })).await {
                    Ok(c) => c,
                    Err(_) => continue, // dead/failing URL → skip
                };
                let messages = vec![
                    ChatMessage::system(&self.system_prompt),
                    ChatMessage::user(format!(
                        "Summarize this web page in one or two sentences. Return STRICT JSON only: \
                         {{\"tldr\": string, \"action_items\": [string]}}.\n\n{fetched}"
                    )),
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
                    Err(_) => continue,
                };
                if let Ok(s) = crate::gate::validate_summary(&out.content) {
                    link_lines.push(format!("- {url}: {}", s.tldr));
                }
            }
        }
        if !link_lines.is_empty() {
            note.body = construct_obsidian::block::upsert_named(
                &note.body,
                "inbox-links",
                &link_lines.join("\n"),
            );
        }

        // --- Step 2: Summarize the note ---
        let summary = {
            let messages = vec![
                ChatMessage::system(&self.system_prompt),
                ChatMessage::user(format!(
                    "Summarize the following note. Return STRICT JSON only: \
                     {{\"tldr\": string, \"action_items\": [string]}}.\n\n{}",
                    note.body
                )),
            ];
            match run_loop(
                self.provider.as_ref(),
                &self.tools,
                messages,
                &LoopConfig { model: self.model.clone(), max_iterations: self.max_iterations },
            )
            .await
            {
                Ok(r) => match crate::gate::validate_summary(&r.content) {
                    Ok(s) => s,
                    Err(e) => return self.fail(run_id, path, &e.to_string()).await,
                },
                Err(e) => return self.fail(run_id, path, &e.to_string()).await,
            }
        };
        note.body = construct_obsidian::block::upsert_named_at_top(
            &note.body,
            "summary",
            &crate::pipelines::summarize::render_summary(&summary),
        );

        // --- Step 3: Tag the note ---
        let existing = construct_obsidian::vault::existing_tags_excluding(
            &self.vault_path,
            &self.exclude_dirs,
        );
        {
            let messages = vec![
                ChatMessage::system(&self.system_prompt),
                ChatMessage::user(format!(
                    "Choose tags for this note. PREFER reusing these existing vault tags when they fit: {}. \
                     Return STRICT JSON only: {{\"tags\": [string]}}.\n\nNOTE:\n{}",
                    existing.join(", "),
                    note.body
                )),
            ];
            match run_loop(
                self.provider.as_ref(),
                &self.tools,
                messages,
                &LoopConfig { model: self.model.clone(), max_iterations: self.max_iterations },
            )
            .await
            {
                Ok(r) => match crate::gate::validate_tags(&r.content, self.max_tags) {
                    Ok(tags) => note.merge_tags(&tags),
                    Err(e) => return self.fail(run_id, path, &e.to_string()).await,
                },
                Err(e) => return self.fail(run_id, path, &e.to_string()).await,
            }
        }

        // --- Step 4: Move decision ---
        let folders = construct_obsidian::vault::list_folders(&self.vault_path, &self.exclude_dirs);
        let proposal = {
            let messages = vec![
                ChatMessage::system(&self.system_prompt),
                ChatMessage::user(format!(
                    "Suggest the single best destination folder for this note. It may be an \
                     existing folder from this list, or a new folder you propose: {}. \
                     Return STRICT JSON only: {{\"destination\": string, \"reason\": string}}.\n\nNOTE:\n{}",
                    folders.join(", "),
                    note.body
                )),
            ];
            match run_loop(
                self.provider.as_ref(),
                &self.tools,
                messages,
                &LoopConfig { model: self.model.clone(), max_iterations: self.max_iterations },
            )
            .await
            {
                Ok(r) => match crate::gate::validate_destination(&r.content) {
                    Ok(p) => p,
                    Err(e) => return self.fail(run_id, path, &e.to_string()).await,
                },
                Err(e) => return self.fail(run_id, path, &e.to_string()).await,
            }
        };

        let note_name = path.file_name().unwrap().to_string_lossy().to_string();
        let is_existing = folders.iter().any(|f| f == &proposal.destination);

        let outcome: String;
        if is_existing {
            // Auto-move into the existing folder. Finalize: status=done, drop run id.
            note.set_str(crate::pipeline::STATUS_KEY, "done");
            note.remove(crate::pipeline::RUN_KEY);
            let finalized = note.to_string();
            let target = self.collision_free_target(&proposal.destination, path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(path, &target)?;
            std::fs::write(&target, &finalized)?;
            self.store.update_status(run_id, RunStatus::Done, None).await?;
            self.store
                .append_event(
                    run_id,
                    "inbox",
                    "moved",
                    serde_json::json!({"to": target.to_string_lossy()}),
                )
                .await?;
            outcome = format!("moved→{}", proposal.destination);
        } else {
            // Recommend only: write block + frontmatter review, mark the run terminal.
            let recommended = inbox::apply_recommendation(
                &note.to_string(),
                &proposal.destination,
                &proposal.reason,
            );
            std::fs::write(path, &recommended)?;
            self.store.update_status(run_id, RunStatus::Done, None).await?;
            self.store
                .append_event(
                    run_id,
                    "inbox",
                    "recommended",
                    serde_json::json!({"destination": proposal.destination}),
                )
                .await?;
            outcome = format!("recommended→{}", proposal.destination);
        }

        // --- Step 5: _index log (best-effort) ---
        let summary_outcome = format!(
            "enriched {} url(s), summarized, tagged, {outcome}",
            link_lines.len()
        );
        if let Some(dir) = path.parent() {
            let index_path = dir.join("_index.md");
            let cur = std::fs::read_to_string(&index_path).unwrap_or_default();
            let updated = inbox::update_index(&cur, &note_name, &summary_outcome);
            if let Err(e) = std::fs::write(&index_path, updated) {
                tracing::warn!("failed to update inbox _index: {e}");
            }
        }
        Ok(())
    }
```

> Note: in the auto-move branch the `_index.md` is written into the note's ORIGINAL parent (the Inbox dir) AFTER the note has been moved out — correct, the log lives in Inbox. `path.parent()` still resolves to the Inbox dir even though `path` no longer exists. Good.

- [ ] **Step 7: Run the tests**

Run: `cargo test -p construct-engine`
Expected: PASS — the two new inbox integration tests AND all existing orchestrator tests (the `collision_free_target` refactor must not change organize behavior).

- [ ] **Step 8: Commit**

```bash
git add crates/construct-engine/src/orchestrator.rs
git commit -m "feat(engine): run_inbox pipeline (enrich/summarize/tag/move-or-recommend/index)" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7: `scan_inbox` directory scanner

**Files:**
- Modify: `crates/construct-engine/src/triggers/idle.rs`, `crates/construct-engine/Cargo.toml`

Lists **top-level** files in the Inbox folder (NOT recursive), applies the shared loop-guard and `should_process_inbox_note`, and returns the paths ready to process. Tested with a temp dir; `filetime` (dev-dependency) sets file mtimes deterministically.

- [ ] **Step 1: Add the dev-dependency**

Run: `cargo add filetime --dev -p construct-engine`
(filetime is small and stable; used only in tests.)

- [ ] **Step 2: Write the failing test** — add to the `tests` module in `triggers/idle.rs`:

```rust
    use crate::guard::is_excluded;
    use std::fs;

    fn set_mtime(path: &std::path::Path, dt: DateTime<Local>) {
        let ft = filetime::FileTime::from_unix_time(dt.timestamp(), 0);
        filetime::set_file_mtime(path, ft).unwrap();
    }

    #[test]
    fn scan_inbox_returns_only_idle_unprocessed_top_level_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let inbox = vault.join("Inbox");
        fs::create_dir_all(&inbox).unwrap();
        fs::create_dir_all(inbox.join("sub")).unwrap();

        let now = Local.with_ymd_and_hms(2026, 6, 2, 12, 0, 0).unwrap();

        // idle + no status → included
        let idle = inbox.join("idle.md");
        fs::write(&idle, "an old idea").unwrap();
        set_mtime(&idle, Local.with_ymd_and_hms(2026, 6, 2, 11, 0, 0).unwrap());

        // too recent → excluded
        let fresh = inbox.join("fresh.md");
        fs::write(&fresh, "just typed").unwrap();
        set_mtime(&fresh, Local.with_ymd_and_hms(2026, 6, 2, 11, 59, 0).unwrap());

        // has construct_status → excluded (no-reprocess)
        let processed = inbox.join("processed.md");
        fs::write(&processed, "---\nconstruct_status: review\n---\nbody").unwrap();
        set_mtime(&processed, Local.with_ymd_and_hms(2026, 6, 2, 8, 0, 0).unwrap());

        // _index is managed → excluded
        let index = inbox.join("_index.md");
        fs::write(&index, "log").unwrap();
        set_mtime(&index, Local.with_ymd_and_hms(2026, 6, 2, 8, 0, 0).unwrap());

        // file in a subfolder → excluded (top-level only)
        let nested = inbox.join("sub").join("nested.md");
        fs::write(&nested, "deep").unwrap();
        set_mtime(&nested, Local.with_ymd_and_hms(2026, 6, 2, 8, 0, 0).unwrap());

        let found = scan_inbox(&inbox, vault, "journal", None, now, 30);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["idle.md".to_string()]);
    }
```

- [ ] **Step 3: Implement `scan_inbox`** — add to `triggers/idle.rs`:

```rust
use crate::guard::is_excluded;
use std::path::{Path, PathBuf};

/// Scan TOP-LEVEL files in `inbox_dir` and return notes ready for Inbox processing:
/// markdown files that are not loop-guarded and pass `should_process_inbox_note`
/// (idle long enough AND no existing `construct_status`). Not recursive.
///
/// `vault_root`/`journal_folder`/`managed_folder` feed the shared loop-guard.
/// `now` comes from the injected clock; `idle_minutes` from config.
pub fn scan_inbox(
    inbox_dir: &Path,
    vault_root: &Path,
    journal_folder: &str,
    managed_folder: Option<&str>,
    now: DateTime<Local>,
    idle_minutes: u64,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(inbox_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Top-level files only — skip directories (not recursive).
        if !path.is_file() {
            continue;
        }
        // Markdown notes only.
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if is_excluded(&path, vault_root, journal_folder, managed_folder) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        let mtime: DateTime<Local> = modified.into();
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        if should_process_inbox_note(&text, mtime, now, idle_minutes) {
            out.push(path);
        }
    }
    out.sort();
    out
}
```

> The existing `idle.rs` already imports `chrono::{DateTime, Local}` and `construct_obsidian::frontmatter::Note`; add the `use` lines above only if not already present (avoid duplicate-import errors). `out.sort()` makes the result deterministic for the test's `assert_eq!`.

- [ ] **Step 4: Run the test**

Run: `cargo test -p construct-engine scan_inbox`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/construct-engine/src/triggers/idle.rs crates/construct-engine/Cargo.toml Cargo.lock
git commit -m "feat(engine): scan_inbox top-level idle note scanner" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 8: Watch-loop wiring — build Inbox orchestrator + spawn idle poller + route `IdleNote`

**Files:**
- Modify: `crates/construct-cli/src/tui/watch_loop.rs`

When `cfg.inbox` is present, build a single Inbox `Orchestrator` (pipeline `Inbox`, rule `"inbox"`), spawn a tokio task that polls `scan_inbox` on an interval and sends `TriggerEvent::IdleNote`, and route `IdleNote` to that orchestrator (with the same per-note lock as tag events). This is integration code; it is validated by the engine tests plus a small construction/route unit test and a manual smoke check.

**Agent resolution:** the Inbox orchestrator's model/provider/tools come from an `Agent`. Resolve in this order: `cfg.inbox.agent` (validated to exist) → the agent of the first rule whose pipeline is `"tag"` or `"summarize"` → the first defined agent. If none exists, log a warning and skip Inbox (the rest of the watcher still runs). The Inbox agent always gets a `web_fetch` tool (URL enrich needs it) even if its `tools` list omits it.

- [ ] **Step 1: Add a routing-helper test** — extend the `tests` module in `watch_loop.rs`:

```rust
    #[test]
    fn route_key_for_idle_note_is_inbox() {
        let ev = TriggerEvent::IdleNote { path: PathBuf::from("/v/Inbox/a.md") };
        assert_eq!(route_key(&ev), RouteTarget::Inbox);
    }
```

- [ ] **Step 2: Extend `RouteTarget` + `route_key`** — change the `Unhandled` arm so `IdleNote` maps to a new `Inbox` target (Scheduled stays `Unhandled` until Plan 3):

```rust
#[derive(Debug, PartialEq)]
enum RouteTarget {
    Tag(String),
    Broadcast,
    Inbox,
    Unhandled,
}

fn route_key(ev: &TriggerEvent) -> RouteTarget {
    match ev {
        TriggerEvent::Tagged { tag, .. } => RouteTarget::Tag(tag.clone()),
        TriggerEvent::StatusChanged { .. } => RouteTarget::Broadcast,
        TriggerEvent::IdleNote { .. } => RouteTarget::Inbox,
        TriggerEvent::Scheduled { .. } => RouteTarget::Unhandled,
    }
}
```

- [ ] **Step 3: Build the Inbox orchestrator** — after the per-rule orchestrator loop (and before `reconcile`), add a block that constructs `Option<Arc<Orchestrator>>` for the inbox. Reuse the existing provider/tools/system_prompt construction pattern from the per-rule loop. Concretely (place near the top of `run_watch`, after `orchestrators` is built):

```rust
    // Build the Inbox orchestrator (Feature A) when [inbox] is configured.
    let inbox_orch: Option<Arc<Orchestrator>> = if let Some(inbox_cfg) = &cfg.inbox {
        // Resolve which agent runs the inbox.
        let agent = inbox_cfg
            .agent
            .as_deref()
            .and_then(|name| cfg.agent(name))
            .or_else(|| {
                cfg.rules
                    .iter()
                    .find(|r| r.pipeline == "tag" || r.pipeline == "summarize")
                    .and_then(|r| cfg.agent(&r.agent))
            })
            .or_else(|| cfg.agents.first());
        match agent {
            None => {
                tracing::warn!("[inbox] configured but no agent available; skipping inbox");
                None
            }
            Some(agent) => {
                let provider: Arc<dyn ModelProvider> =
                    Arc::new(OllamaProvider::new(agent.base_url.clone()));
                // Inbox always needs web_fetch for URL enrichment.
                let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
                tools.insert("web_fetch".into(), Arc::new(WebFetch::new()));
                if agent.tools.iter().any(|t| t == "web_search") {
                    if let Some(ws) = &cfg.tools.web_search {
                        let key = std::env::var(&ws.api_key_env).unwrap_or_default();
                        tools.insert("web_search".into(), Arc::new(WebSearch::tavily(key)));
                    }
                }
                let system_prompt = agent
                    .system_prompt_file
                    .as_ref()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .unwrap_or_else(|| {
                        format!("You are {}, organizing inbox notes. Always answer with strict JSON.", agent.name)
                    });
                Some(Arc::new(Orchestrator {
                    store: store.clone(),
                    provider,
                    tools,
                    model: agent.model.clone(),
                    agent: agent.name.clone(),
                    rule: "inbox".into(),
                    pipeline: PipelineKind::Inbox,
                    system_prompt,
                    max_iterations: 8,
                    done_tag: None,
                    vault_path: shellexpand_tilde(&cfg.vault.path).into(),
                    max_tags: cfg.actions.tag.max_tags,
                    exclude_dirs: cfg.actions.organize.exclude_dirs.clone(),
                }))
            }
        }
    } else {
        None
    };
```

- [ ] **Step 4: Reconcile the inbox orchestrator** — in the existing reconcile loop, also reconcile the inbox orchestrator if present:

```rust
    if let Some(o) = &inbox_orch {
        if let Err(e) = o.reconcile().await {
            tracing::warn!("inbox reconcile failed: {e}");
        }
    }
```

- [ ] **Step 5: Spawn the idle poller** — after the `watch(...)` debouncer is created and `tx` is cloned, spawn a polling task when inbox is configured. Add (using a clone of `tx`):

```rust
    // Idle poller: scan top-level Inbox files on an interval; emit IdleNote events.
    if let Some(inbox_cfg) = cfg.inbox.clone() {
        let tx_idle = tx.clone();
        let vault_root: std::path::PathBuf = shellexpand_tilde(&cfg.vault.path).into();
        let inbox_dir = vault_root.join(&inbox_cfg.folder);
        let journal_folder = cfg
            .journal
            .as_ref()
            .map(|j| j.folder.clone())
            .unwrap_or_else(|| "journal".to_string());
        let managed = cfg.vault.managed_folder.clone();
        let idle_minutes = inbox_cfg.idle_minutes;
        tokio::spawn(async move {
            use construct_core::clock::{Clock, SystemClock};
            let clock = SystemClock;
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                ticker.tick().await;
                let now = clock.now_local();
                let ready = construct_engine::triggers::idle::scan_inbox(
                    &inbox_dir,
                    &vault_root,
                    &journal_folder,
                    managed.as_deref(),
                    now,
                    idle_minutes,
                );
                for path in ready {
                    let _ = tx_idle.send(VaultEvent::NoteTagged {
                        // NOTE: see Step 6 — IdleNote needs its own channel/event.
                        path,
                        tag: String::new(),
                    });
                }
            }
        });
    }
```

> **IMPORTANT — channel type:** the existing `tx`/`rx` channel carries `VaultEvent`, but the idle poller needs to deliver `TriggerEvent::IdleNote`. Do NOT smuggle idle notes through `VaultEvent::NoteTagged` (that would route them as `Tag("")`). Instead, change the watch-loop channel to carry `TriggerEvent` end-to-end:
> - Create the channel as `mpsc::unbounded_channel::<TriggerEvent>()`.
> - Wrap the `watch(...)` debouncer so the watcher's `VaultEvent`s are mapped to `TriggerEvent` before being sent. The simplest approach: keep `watch()` sending `VaultEvent` on its own channel `rx_vault`, then spawn a tiny forwarder task `while let Some(ev) = rx_vault.recv().await { let _ = tx.send(ev.into()); }`. Now `tx` is the `TriggerEvent` sender shared by the forwarder and the idle poller.
> - The idle poller then sends `TriggerEvent::IdleNote { path }` directly.
> - The main `while let Some(event) = rx.recv().await` loop now receives `TriggerEvent` directly (no `.into()` needed) and uses `route_key(&event)` as before; for the `Tag`/`Broadcast` paths it must reconstruct the `VaultEvent` to pass to `o.handle(...)`, OR (preferred) add an `Orchestrator::handle` overload. Keep it simple: in the loop, `match event` to extract the path and call the right orchestrator method directly:
>   - `RouteTarget::Tag(tag)` → `orchestrators.get(&tag)` → spawn `o.handle(VaultEvent::NoteTagged { path, tag })` (rebuild the VaultEvent from the TriggerEvent fields).
>   - `RouteTarget::Broadcast` → for the `TriggerEvent::StatusChanged { path, status }`, broadcast `o.handle(VaultEvent::StatusChanged { path, status })`.
>   - `RouteTarget::Inbox` → if `let Some(o) = &inbox_orch`, acquire the per-note lock and spawn `o.handle_idle(&path)`.
>   - `RouteTarget::Unhandled` → debug log.

- [ ] **Step 6: Rewrite the event loop** for the `TriggerEvent` channel per the note above. Concretely:

```rust
    while let Some(event) = rx.recv().await {
        match route_key(&event) {
            RouteTarget::Tag(tag) => {
                if let (Some(o), TriggerEvent::Tagged { path, tag: t }) =
                    (orchestrators.get(&tag), event.clone())
                {
                    let o = o.clone();
                    let lock = lock_for(&note_locks, &path);
                    tokio::spawn(async move {
                        let _guard = lock.lock().await;
                        if let Err(e) = o.handle(VaultEvent::NoteTagged { path, tag: t }).await {
                            tracing::error!("handler error: {e}");
                        }
                    });
                }
            }
            RouteTarget::Broadcast => {
                if let TriggerEvent::StatusChanged { path, status } = event {
                    for o in orchestrators.values() {
                        let o = o.clone();
                        let (p, s) = (path.clone(), status.clone());
                        tokio::spawn(async move {
                            let _ = o.handle(VaultEvent::StatusChanged { path: p, status: s }).await;
                        });
                    }
                }
            }
            RouteTarget::Inbox => {
                if let (Some(o), TriggerEvent::IdleNote { path }) = (inbox_orch.as_ref(), event) {
                    let o = o.clone();
                    let lock = lock_for(&note_locks, &path);
                    tokio::spawn(async move {
                        let _guard = lock.lock().await;
                        if let Err(e) = o.handle_idle(&path).await {
                            tracing::error!("inbox handler error: {e}");
                        }
                    });
                }
            }
            RouteTarget::Unhandled => {
                tracing::debug!("unhandled trigger event: {event:?}");
            }
        }
    }
```

with a small helper added at module scope to dedupe the per-note lock acquisition (replaces the inline `note_locks.lock().unwrap()...` blocks):

```rust
use tokio::sync::Mutex as AsyncMutex;
type NoteLocks = Arc<std::sync::Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>;

fn lock_for(locks: &NoteLocks, path: &std::path::Path) -> Arc<AsyncMutex<()>> {
    let key = path.to_string_lossy().to_string();
    let mut map = locks.lock().unwrap();
    map.entry(key).or_insert_with(|| Arc::new(AsyncMutex::new(()))).clone()
}
```

> Adjust the `note_locks` declaration to the `NoteLocks` type alias. Ensure the `route_key`/`RouteTarget::Tag` behavior remains identical to Plan 1 for tag/status events (same per-note lock, same broadcast).

- [ ] **Step 7: Build + test + manual smoke**

Run: `cargo test --workspace && cargo build`
Expected: PASS. Then `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt --all -- --check` clean (run `cargo fmt --all` first).

Manual smoke check (document the result; do not block on Ollama availability):
```bash
# With an [inbox] table in construct.toml pointing at a test vault, create a stale note:
#   echo "test note https://example.com" > <vault>/Inbox/smoke.md
#   touch -d '1 hour ago' <vault>/Inbox/smoke.md
# Run `entertheconstruct` (the watch loop) and confirm within ~60s the note gets a
# summary block + tags + either a move or a recommendation, and Inbox/_index.md logs it.
```

- [ ] **Step 8: Commit**

```bash
git add crates/construct-cli/src/tui/watch_loop.rs
git commit -m "feat(cli): wire Inbox orchestrator + idle poller + IdleNote routing" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Final verification (after all tasks)

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

All green. Confirm: existing organize/tag/summarize/research tests still pass (the `collision_free_target` refactor and the `TriggerEvent` channel change are behavior-preserving for tag/status); the two `run_inbox` integration tests pass; `scan_inbox` test passes.

## Self-Review checklist (done at plan-write time)
- **Spec coverage:** top-level-Inbox-only scan ✅ (Task 7, `is_file` + no recursion + `is_excluded`); idle ≥ idle_minutes by mtime ✅ (Plan 1 core, used by 7); URL enrich first-5 skip-on-fail ✅ (Tasks 3, 6); reuse `apply_summary` rendering ✅ (6 uses `render_summary`+`upsert_named_at_top`); reuse `apply_tags`/`merge_tags` ✅ (6 uses `merge_tags`); move only into existing folder else recommend at top ✅ (6, `validate_destination` + existing-folder branch + `apply_recommendation`); never create folders / never move into non-existing ✅ (6, move only when `is_existing`); `_index` managed-block log ✅ (Tasks 4, 6); no-reprocess (skip notes with `construct_status`) ✅ (Plan 1 core in `scan_inbox`); re-run by clearing status ✅ (store run marked Done — see header).
- **Type consistency:** `read_named`, `validate_destination`, `extract_urls`, `apply_recommendation`, `update_index`, `scan_inbox`, `run_inbox`, `handle_idle`, `collision_free_target`, `RouteTarget::Inbox`, `lock_for`/`NoteLocks` are used consistently. `InboxCfg.agent` added additively.
- **No placeholders:** every code step contains real code. The one genuinely tricky step (Task 8 channel-type change) is spelled out with the forwarder-task approach.
- **Reuse over reimplementation:** summary rendering, tag merge, move-with-collision (extracted + shared), `WebFetch`, `run_loop`, gates, managed blocks, `existing_tags_excluding`, `list_folders`, the claim/idempotency path (`handle_idle`→`handle_tagged`), `Store`, `construct_status` lifecycle — all reused.
