# The Construct — Slice 1 Design Spec

**Date:** 2026-05-29
**Status:** Approved (design) — pending spec review
**Author:** Matt (with Claude Code)

---

## 1. Vision & Scope

**The Construct** is a local-first agent runtime that runs on the user's home
machine/network. Its guiding principle is **deterministic first**: a
deterministic outer shell wraps a single non-deterministic (agentic) step. The
shell owns all side effects; the agent only proposes data.

This document specifies **Slice 1** — the first runnable vertical. The overall
product (model abstraction, plugin tools, vector DB, fancy TUI, Google
Drive/Notion/Obsidian integrations) is larger and will be specced in later
slices. Slice 1 is intentionally a complete end-to-end path through the core
architecture so every later feature has a proven foundation to build on.

### In scope (Slice 1)
- Cargo workspace skeleton and core traits.
- Config system (TOML).
- SQLite persistence (behind a `Store` trait).
- A *named* agent defined in config ("nameable agent" requirement).
- Obsidian vault watcher (deterministic entrypoint).
- Deterministic tag → pipeline routing.
- A model layer (`ModelProvider` trait) with a **local Ollama** implementation.
- An agentic **web-research** loop with `web_search` + `web_fetch` tools.
- Deterministic write-back into the note + frontmatter status state machine.
- Human accept/reject via a frontmatter status field.
- A styled CLI/TUI launched via the `entertheconstruct` command, including a
  live dashboard **and an interactive chat pane** for talking to the local
  model directly.

### Explicitly deferred (later slices)
- Vector database / vault RAG.
- Frontier-model wiring (the `ModelProvider` trait must accommodate it, but only
  Ollama is implemented now).
- Google Drive, Notion integrations.
- Fully user-authored pipelines in TOML (Slice 1 ships built-in pipelines
  selected by name).
- Postgres (the `Store` trait must accommodate it; only SQLite is implemented).
- Self-hosted SearXNG search backend (design for it; ship Tavily/Brave first).

---

## 2. Confirmed Decisions

| Decision | Choice |
| --- | --- |
| First slice scope | Foundation + Obsidian watcher (full vertical) |
| First action behavior | Deterministic file ops **+ AI from day one** |
| Research scope | **Web research** (no vector DB this slice) |
| Approval flow | **Frontmatter status field** state machine |
| Default model backend | **Local first** via **Ollama** (abstraction supports frontier) |
| Web search | **Search API (Tavily/Brave)** first; SearXNG later |
| Architecture | **A: Config-driven pipeline** (SQLite-persisted state machine) |
| TUI scope | Live dashboard **+ interactive chat with the local model** |
| Pipelines | Built-in (selected by name in config) for Slice 1 |
| Workspace | Multi-crate cargo workspace |

Open detail to confirm during implementation: the exact Ollama **model name**
and `base_url` (placeholder: `qwen2.5:14b` @ `http://<host>:11434`).

---

## 3. Architecture

### 3.1 Core concepts
- **Rule** — maps a trigger (tag pattern) to a pipeline + agent. Declared in
  TOML.
- **Pipeline** — an ordered list of **Stages**.
- **Stage** — either *deterministic* (a built-in op) or *agent* (delegates to
  the agentic loop). Each stage is a typed, independently testable unit.
- **Engine / Orchestrator** — consumes vault events, finds the matching rule,
  and runs the pipeline as a **SQLite-persisted state machine** (resumable and
  observable across restarts).
- **Watcher** — filesystem watcher over the vault; parses frontmatter + tags;
  emits `NoteTagged` / `StatusChanged` events. Debounced, because Obsidian
  writes a file multiple times per save.
- **ModelProvider** (trait) — Ollama implementation now (OpenAI-compatible
  endpoint); frontier providers later via the same trait.
- **Tool** (trait) — `web_search` (Tavily/Brave) and `web_fetch` now; adding
  tools is the primary expandability mechanism.
- **Store** (trait) — SQLite (sqlx) now; Postgres later via the same trait.
- **Agent** — a *named* agent (e.g. "Scout") with a domain, model config,
  allowed tools, and system prompt. Fully config-defined.

### 3.2 Determinism boundary (the core invariant)
The agent **never performs side effects**. The agent stage returns *structured
data only*; every file mutation, frontmatter change, move, and DB write is
performed by a deterministic stage. This makes "agent proposes, shell disposes"
a structural guarantee, not a convention.

---

## 4. The Research Pipeline (concrete state machine)

Trigger: a note gains the tag `#theconstruct/research`.

State machine: `queued → researching → review → (accepted | rejected) → done`,
with an `error` terminal-ish state. Run state is persisted in SQLite so a
restart resumes mid-pipeline (see §4.3 for the crash-recovery rule).

| # | Stage | Type | Behavior |
| --- | --- | --- | --- |
| 1 | `claim` | deterministic | Set frontmatter `construct_status: queued` + `construct_run_id`; persist run in SQLite; idempotency guard so the engine ignores its own subsequent writes. |
| 2 | `research` | agent | Run the named agent on the local Ollama model with tools `[web_search, web_fetch]`. Input = note title/body (+ optional `construct_query` field). Output = structured result (summary, findings, sources). Bounded by max tool-iterations, timeout, and token budget. |
| 3 | `gate` | deterministic | Validate the agent output against the schema **and** a grounding check (see §4.1). Invalid, ungrounded, or over-budget → `status: error` + message. (The "checker," deterministic in v1.) |
| 4 | `write_back` | deterministic | Write results into a managed block (`<!-- construct:research:start -->…<!-- construct:research:end -->`) so re-runs replace cleanly; set `status: review`. |
| 5 | `await_decision` | deterministic (pausing) | Run parks in `review` status in SQLite. A `StatusChanged` event resumes it when the user sets the status field to `accepted` or `rejected`. |
| 6 | `finalize` | deterministic | **accepted** → set `status: done`, add `#theconstruct/done`, strip transient fields, mark run complete. **rejected** → remove managed block, set `status: rejected`. **v1 does NOT move the note** (see §4.2). |

### 4.1 Output schema + grounding (the gate)

The gate is what makes an unreliable local model safe. It enforces two things,
both deterministic:

1. **Shape.** The agent's final message must contain a JSON object matching
   `ResearchResult`: a non-empty `summary` (string), a non-empty `findings`
   (array of strings), and a non-empty `sources` (array of `{title, url}`).
   JSON may be embedded in prose or a ```json fence; the gate extracts the first
   balanced object.
2. **Grounding.** Every `source.url` must appear in the **evidence** the agent
   actually gathered — the concatenation of all tool outputs plus every URL
   passed to `web_fetch` during the run. A source the model invented (not
   present in any tool result) fails the gate. This is enforceable precisely
   *because* the tool layer is deterministic and owned by the shell. Findings
   grounding (tying each finding to a source) is a softer future check; v1
   grounds sources only.

A failed gate sets `status: error` with a message naming the failure (bad JSON,
empty field, or ungrounded URL) and never writes a research block.

### 4.2 Frontmatter contract

- The status field is the flat key **`construct_status`** (chosen over a nested
  `construct.status` for robust, simple YAML editing). The run id is
  **`construct_run_id`**.
- Engine-written values: `queued`, `researching`, `review`, `done`, `rejected`,
  `error`. User-settable values (the *only* ones the watcher acts on as a
  decision): exactly `accepted` or `rejected` (lowercase).
- **Invalid/typo'd values are a no-op in v1** (e.g. `accpeted` does nothing).
  This is a known v1 limitation; inline validation feedback is a later slice.
- The watcher only emits `NoteTagged` when `construct_status` is *absent*, and
  only emits `StatusChanged` for `accepted`/`rejected`. Because every
  engine-written value is none of those, the engine's own writes never
  re-trigger the pipeline — this is the deterministic "ignore-self" mechanism
  (combined with debouncing).

### 4.3 Crash recovery / resume

Deterministic stages are idempotent and resume naturally. The one
non-idempotent stage is `research` (web calls, tokens). Recovery rule: on
startup the engine **reconciles** any run left in `queued` or `researching`
(i.e. crashed mid-flight) by marking it `error` and re-triggering the note from
the top. Runs parked in `review` are left alone — they are correctly waiting for
the human. This keeps "resume across restarts" honest without trying to resume a
half-finished LLM call.

---

## 5. Configuration (TOML)

```toml
[construct]
name = "The Construct"

[vault]
path = "~/ObsidianVault"
managed_folder = "Construct"      # finalize destination (optional)

[[agents]]
name = "Scout"                    # nameable agent
domain = "research"
provider = "ollama"
model = "qwen3.6:27b"            # local Ollama; swap to qwen3:4b-instruct-2507-q8_0 for speed
base_url = "http://192.168.1.33:11434"
tools = ["web_search", "web_fetch"]
system_prompt_file = "prompts/scout.md"

[tools.web_search]
backend = "tavily"               # later: "searxng"
api_key_env = "TAVILY_API_KEY"

[[rules]]
match_tag = "theconstruct/research"
agent     = "Scout"
pipeline  = "research"           # built-in pipeline id
```

Config is the source of truth for agents and rules. Pipelines are built-in and
selected by name in Slice 1. Secrets are referenced by environment variable
name, never stored in config.

---

## 6. CLI / Entrypoint

- Installed as the `entertheconstruct` binary.
- **No args** → launches the styled TUI (ratatui).
- TUI layout:
  - Header: agent/vault + earthy-blue/brown theme.
  - **Dashboard**: watcher status, active-runs table, scrolling event log.
  - **Chat pane**: interactive, streaming conversation with the configured local
    model (uses `ModelProvider` directly; pure chat, no tools in v1).
  - Footer: keybindings.
- Subcommands: `watch` (run the watcher, foreground/daemon), `status`, `runs`,
  `config check`, `init`.
- A small `theme` module holds the earthy blue/brown palette, shared by the TUI
  and plain CLI output.

---

## 7. Cargo Workspace

| Crate | Responsibility |
| --- | --- |
| `construct-core` | Domain types + traits (`ModelProvider`, `Tool`, `Stage`, `Store`, events) |
| `construct-config` | TOML load + validation (serde) |
| `construct-store` | SQLite via sqlx, migrations (impl of `Store`) |
| `construct-model-ollama` | `ModelProvider` for Ollama (OpenAI-compatible) |
| `construct-tools` | `web_search` (Tavily/Brave) + `web_fetch` tools |
| `construct-obsidian` | Watcher (`notify`), frontmatter parsing, markdown block management |
| `construct-engine` | Orchestrator, rule matching, pipeline state machine, agentic loop |
| `construct-cli` | `entertheconstruct` binary, TUI (ratatui), theme |

Rationale: separate crates for model providers and tools keep "add a new
provider / tool" changes isolated, directly serving the expandability
requirement. If ceremony becomes a drag, `construct-tools` and
`construct-model-ollama` can later fold into `construct-engine` without changing
the public traits.

---

## 8. Data Model (SQLite)

- `runs` — `id`, `rule`, `agent`, `note_path`, `status`, `created_at`,
  `updated_at`, `error`
- `run_events` — `id`, `run_id`, `stage`, `event`, `payload_json`, `ts`
  (audit + observability; powers the TUI event log)

The DB holds only runtime state. No vector tables in this slice. Schema is
applied via migrations so a Postgres `Store` impl can reuse them later.

---

## 9. Error Handling

- Every stage returns `Result`; a failure sets frontmatter `status: error` with
  a message and persists a `run_event`. The watcher never crashes on a single
  note's failure.
- Idempotency via run-claim + content markers; the engine ignores file events it
  itself caused.
- File events are debounced (Obsidian saves fire multiple writes).
- The agent loop is bounded by max iterations, a timeout, and a token budget.
- Malformed notes (bad YAML, missing fields) are logged as events and skipped,
  not fatal.

---

## 10. Testing Strategy

- **Unit tests** per deterministic stage (`claim`, `gate`, `write_back`,
  `finalize`) using temp directories.
- **Watcher tests** driven by simulated filesystem events.
- **Full pipeline test** using a **mock `ModelProvider` + mock `Tool`s** —
  exercises the entire state machine with no network or LLM. This is the
  primary regression guard and a direct payoff of the trait-based design.
- **Optional live integration test** against a real Ollama, behind a cargo
  feature flag, excluded from default CI.

---

## 11. How Slice 1 Maps to Future Work

- New tools → implement `Tool`, register, reference in config. (Expandability.)
- Frontier models → implement `ModelProvider`. (Model choice.)
- Vault RAG → add a vector store + a `vault_search` tool; reuse the same agent
  loop.
- Notion / Drive → new watchers/triggers + new deterministic stages.
- User-authored pipelines → promote built-in pipelines to a TOML stage list.
- Postgres → new `Store` impl; migrations already exist.
- Richer TUI → build on the dashboard + chat foundation.
