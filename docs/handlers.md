# Handlers

A **handler** is what The Construct does with a note. Routing is deliberately
simple and deterministic-first.

## How routing works

```
note → tag → rule → pipeline → Priori (judge) → Determa (deterministic)
                                              ↘ escalate to a model (last resort)
```

1. **Tag → rule.** A note carries an inline trigger tag (e.g.
   `#theconstruct/remind-me`). Each `[[rules]]` entry maps a `match_tag` to a
   `pipeline` and the `agent` that pipeline should use if it needs one.
2. **Pipeline.** The pipeline name resolves to a built-in `PipelineKind`. This is
   the unit of work — `remind-me`, `file-this`, `research-this`, and a few more.
3. **Priori judges.** Before any model is contacted, `priori::judge` decides
   whether the pipeline can be handled by deterministic code or must escalate.
   Its verdict is `Decision::Deterministic(reason)` or `Decision::Escalate(reason)`,
   and the reason is recorded for the activity log and audit trail.
   - `remind-me` is **always** deterministic.
   - `file-this` is deterministic *iff* a `[actions.file_this]` keyword rule
     matches; otherwise it escalates to a local classifier model.
   - `research-this` (and `summarize`, `tag`, `organize`) genuinely need
     reasoning, so they escalate.
4. **Determa executes** the deterministic path, or the orchestrator runs the
   agent loop for the escalation path. Either way, results are written back into
   the note atomically and the run is logged to the SQLite run store.

This keeps the thesis — *most of your agent calls didn't need to be model calls*
— as an explicit, testable decision rather than an accident of control flow.

## Extending without code

For most needs you don't write Rust. Two no-code extension points:

### 1. Config rules (tag → pipeline)

Wire any trigger tag to any built-in pipeline by adding a `[[rules]]` entry. Want
a `#work/research` tag to run research-this through a cloud agent? Add a rule.
This is the primary way to customize behavior.

```toml
[[rules]]
match_tag = "work/research"
agent = "CloudScout"
pipeline = "research-this"
```

### 2. file-this keyword rules

The `file-this` handler's entire deterministic tier is config. Add keyword→folder
rules and matching notes are filed with no model call:

```toml
[actions.file_this]
rules = [
  { any_of = ["meeting", "1:1", "standup"], folder = "Work/Meetings" },
  { any_of = ["recipe", "ingredients"], folder = "Kitchen" },
]
```

## Authoring a new handler (in code)

Handlers are **built-in pipelines**, not a dynamic plugin system. That's a
deliberate "stay small" choice: a handler is a typed, reviewed Rust pipeline, not
arbitrary code or a model emitting shell commands. Adding one is four small, local
edits. The cleanest model to copy is `remind-me` — a self-contained, fully
deterministic handler with no model and no network.

A handler has four touch points:

1. **A `PipelineKind` variant** — the enum case for your handler.
2. **A pure pipeline module** — the actual logic, written as pure transforms over
   note text so it's trivially unit-testable.
3. **An orchestrator match arm** — wires the variant to your module and records
   the run.
4. **A `KNOWN_PIPELINES` entry** — so configs can name it and validation accepts it.

### Worked example: how `remind-me` is built

**1. The pure pipeline module** — `crates/construct-engine/src/pipelines/remind.rs`.
Everything is a pure function over strings; no I/O, no model. The two entry points
the orchestrator calls:

```rust
// Parse "remind me to <task> [<when>]" out of note text. None if no instruction.
pub fn parse_reminder(body: &str, now: DateTime<Local>) -> Option<Reminder> { … }

// Apply the reminder to the note text: managed block at top + frontmatter. Pure.
pub fn apply_reminder(
    text: &str,
    r: &Reminder,
    captured: NaiveDate,
    done_tag: Option<&str>,
) -> String { … }
```

Because these are pure, the handler's whole behavior is covered by ordinary unit
tests (`parse_reminder("remind me to call mom tomorrow", now)` → `task = "call mom"`,
due tomorrow 09:00). No mock vault, no mock model.

**2. The `PipelineKind` variant** — `crates/construct-engine/src/pipelines/mod.rs`:

```rust
pub enum PipelineKind {
    RemindMe,   // deterministic: parse and record. NEVER calls a model.
    FileThis,
    Research,
    // …
}

impl PipelineKind {
    pub fn from_name(name: &str) -> Option<PipelineKind> {
        match name {
            "remind-me" | "remind_me" => Some(PipelineKind::RemindMe),
            // …
        }
    }
    // Marks this kind as running with zero model calls — the thesis, made checkable.
    pub fn is_deterministic(&self) -> bool {
        matches!(self, PipelineKind::RemindMe)
    }
}
```

**3. Priori's verdict** — `crates/construct-engine/src/priori.rs`. For a purely
deterministic handler this is one line:

```rust
PipelineKind::RemindMe => Decision::Deterministic("remind-me is rule-based".into()),
```

A handler that's deterministic *sometimes* (like `file-this`) returns
`Decision::Escalate(...)` on the path that needs a model.

**4. The orchestrator match arm** — `crates/construct-engine/src/orchestrator.rs`.
The dispatch table calls your runner:

```rust
match self.pipeline {
    PipelineKind::RemindMe => self.run_remind(&run_id, path, &original).await,
    // …
}
```

…and the runner is deliberately small. Note it never touches `self.provider`:

```rust
async fn run_remind(&self, run_id: &RunId, path: &Path, original: &str) -> anyhow::Result<()> {
    self.store.update_status(run_id, RunStatus::Running, None).await?;
    let note = Note::parse(original);
    let Some(reminder) = remind::parse_reminder(&note.body, now) else {
        return self.fail(run_id, path, "no \"remind me to …\" instruction found").await;
    };
    let current = std::fs::read_to_string(path)?;          // re-read to preserve edits
    let applied = remind::apply_reminder(&current, &reminder, today, self.done_tag.as_deref());
    write_atomic(path, &applied)?;                          // atomic write — never corrupts
    self.store.update_status(run_id, RunStatus::Done, None).await?;
    self.store.append_event(run_id, "remind", "done",
        serde_json::json!({ "deterministic": true, "task": reminder.task })).await?;
    Ok(())
}
```

**5. Register the name** — `crates/construct-config/src/lib.rs`, in
`KNOWN_PIPELINES`, so a rule can name your pipeline and config validation accepts
it:

```rust
pub const KNOWN_PIPELINES: &[&str] = &[
    "remind-me", "file-this", "research-this", /* … your new name … */
];
```

### Conventions to follow

- **Pure transforms.** Keep the logic in the pipeline module as pure functions
  over note text. The orchestrator handles I/O, claiming the run, and logging.
- **Atomic writes only.** Write through `write_atomic` (write a temp sibling,
  then rename). Never truncate-then-write — a crash must never leave a half-written
  note.
- **Re-read before writing back.** Read the current file contents inside the
  runner so a user edit made while the run was in flight isn't clobbered.
- **Deterministic-first.** If your handler *can* answer without a model, make
  Priori say so and skip the agent. Reach for the model only for the irreducible
  reasoning step.
- **Escalation handlers** run the agent loop and pass the model's output through a
  validating **gate** (`crates/construct-engine/src/gate.rs`) before writing
  anything back — the model's response is untrusted until it's shape-checked (and,
  for research, grounded against the evidence it gathered).
- **Prompts live in files**, not inlined in Rust. Point an agent at a
  `system_prompt_file`; the templates ship in `prompts/` and are user-overridable.
