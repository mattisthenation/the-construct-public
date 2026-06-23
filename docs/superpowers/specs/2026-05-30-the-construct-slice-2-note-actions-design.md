# The Construct — Slice 2: Note Actions Design Spec

**Date:** 2026-05-30
**Status:** Approved (design) — pending spec review
**Author:** Matt (with Claude Code)
**Builds on:** [Slice 1 Design](2026-05-29-the-construct-slice-1-design.md), [Slice 2 Backlog](../slice-2-backlog.md)

---

## 1. Vision & Scope

Slice 1 proved the deterministic-first pipeline end-to-end with one action
(research). Slice 2 adds three **local-model-friendly** note actions that play to
the strengths of a local model (short, structured transforms over a single
note's content rather than long multi-step web research):

1. **Summarize** — prepend a TL;DR + action items to a note.
2. **Tag** — identify and add appropriate tags to a note.
3. **Organize** — move a note to the appropriate folder.

The enabling change is a small refactor: generalize the orchestrator from a
single hardcoded pipeline (research) to **dispatch by `rule.pipeline`**, so any
tag can route to any action. Research is preserved unchanged as one pipeline
among several.

### In scope (Slice 2)
- **Phase 0 — multi-pipeline dispatch refactor.** Orchestrator resolves the
  `Rule` (and thus pipeline + agent) per event from config. Research behavior
  and tests unchanged.
- **Phase 1 — Summarize** (auto-apply).
- **Phase 2 — Tag** (auto-apply).
- **Phase 3 — Organize** (review gate; the only file-moving action).
- One tag per action; serialize multiple actions per note.
- All three use the configured local Ollama model.

### Explicitly deferred (later slices / backlog)
- `claude -p` (Claude CLI) `ModelProvider` for research — see Appendix A.
- Multiple cooperating local models (e.g. a parser model + a JSON model) — see
  Appendix B.
- Robustness items from `slice-2-backlog.md` (JSON repair loop, configurable
  iteration budget, panic hardening at the spawn boundary, watch-mode progress
  UX). Related but tracked separately; not built here.
- A `#theconstruct/tidy` combo tag that chains actions.
- Parallel execution of multiple actions on the same note.
- Vector DB / RAG, Notion, Google Drive, Postgres (still future).

---

## 2. Confirmed Decisions

| Topic | Decision |
| --- | --- |
| Approval model | **Auto-apply** summarize + tag; **review gate** for organize (file move) |
| Organize folder source | **Scan live vault folders** at run time; deterministic guard rejects any pick not in the scanned set |
| Tag vocabulary | **Prefer existing vault tags**; model may add new, but is steered to reuse; count capped + normalized |
| Triggering | **One tag per action** (`#theconstruct/summarize`, `/tag`, `/organize`); compose by applying several |
| Concurrency | **Serialize per note** — one action at a time per note; others queue |
| Safety | **Managed blocks + idempotent merge + done tag**; organize records pre-move path for undo |
| Model | **Configured local model** (per-agent override already supported in config) |
| Scope/plan | **One spec, phased plan** (Phase 0 → 1 → 2 → 3) |

---

## 3. Architecture — Phase 0: multi-pipeline dispatch

### 3.1 Today (Slice 1)
`Orchestrator` holds a single `rule`, `agent`, `system_prompt`, `done_tag`, and
`handle_tagged` always runs the research flow inline. The watcher already passes
the matched tag in `VaultEvent::NoteTagged { path, tag }`, but the tag is ignored
for routing.

### 3.2 Target
- `Orchestrator` holds the whole `Config` (or a resolved registry) instead of one
  rule's fields. On `NoteTagged { path, tag }`:
  1. `rules::match_rule(cfg, tag)` → `Rule { match_tag, agent, pipeline }`.
  2. Look up the named `Agent` (model, base_url, tools, system prompt).
  3. Dispatch to the pipeline named by `rule.pipeline`.
- A **`Pipeline` trait** (or enum dispatch) gives each action a common shape:

  ```
  claim → run (agent) → gate (deterministic validate) → apply → finalize
  ```

  Auto-apply pipelines (summarize, tag) go `apply → done` in one pass. The
  review pipelines (research, organize) `apply` a *proposal* and park in
  `review` until a `StatusChanged` decision.
- **Reconcile** generalizes: a stale `queued`/`researching`/`running` run is
  re-triggered for whatever pipeline its rule names (not assumed to be research).
- Research is refactored into `pipelines::research` with **no behavior change**;
  its Slice 1 tests must still pass verbatim.

### 3.3 Pipeline registry
Built-in pipelines are selected by name (Slice 1 decision preserved — no
user-authored pipelines yet). Unknown `pipeline` name in config → config
validation error (extend `Config::validate`).

### 3.4 Status vocabulary
Reuse Slice 1's `construct_status` state machine. Add one running state name
shared by auto-apply actions:

- Auto-apply: `queued → running → done` (or `error`).
- Review: `queued → running → review → accepted|rejected → done` (or `error`).

(`researching` remains a valid alias used by the research pipeline to avoid
breaking Slice 1; new pipelines use `running`.)

---

## 4. The Three Actions

All actions: agent returns **structured JSON only**; deterministic stages own
every file mutation (the Slice 1 invariant). Each has a deterministic **gate**.

### 4.1 Summarize (auto-apply)
- **Input:** note body (frontmatter stripped).
- **Agent output:** `{ "tldr": string, "action_items": [string] }` (action_items
  may be empty).
- **Gate:** valid JSON; non-empty `tldr`.
- **Apply:** write a managed block at the **top of the body** (immediately after
  frontmatter), markers `<!-- construct:summary:start -->` …
  `<!-- construct:summary:end -->`, idempotent (re-run replaces). Rendered as:

  ```
  > [!summary] TL;DR
  > <tldr>
  >
  > **Action items**
  > - [ ] <item>
  ```
- **Finalize:** `status: done`, add `#theconstruct/done`. No review.

### 4.2 Tag (auto-apply)
- **Input:** note body + **existing vault tag set** (gathered by scanning all
  notes' frontmatter `tags:` and inline `#tags`, deduped).
- **Agent output:** `{ "tags": [string] }`.
- **Gate:** valid JSON; normalize each tag (lowercase, strip leading `#`,
  spaces→`-`); drop empties; **cap at N (default 8)**; dedupe. Prefer-existing is
  a prompt steer, not a hard constraint (new tags allowed).
- **Apply:** **merge** normalized tags into frontmatter `tags:` — union with
  existing, never removing or duplicating. Frontmatter list form.
- **Finalize:** `status: done`, add `#theconstruct/done`.

### 4.3 Organize (review gate)
- **Input:** note body + **live folder list** (relative dirs under the vault
  root, excluding dotfolders and the DB).
- **Agent output:** `{ "destination": string, "reason": string }`.
- **Gate:** valid JSON; **`destination` MUST be one of the scanned folders**
  (deterministic guard) — else `error`. Reject a no-op (destination == current
  dir) by completing as `done` with a "already organized" note rather than a
  move.
- **Apply (proposal):** write `construct_proposed_move: <destination>` and
  `construct_move_reason: <reason>` to frontmatter; `status: review`. The note
  does **not** move yet.
- **Decision:**
  - `accepted` → move the file to `<destination>/<filename>`; record
    `construct_moved_from: <original path>` in frontmatter and a `run_event`;
    `status: done`. Handle name collisions (append ` (1)` etc.).
  - `rejected` → strip the proposal fields; `status: rejected`; no move.
- This is the only action that moves files, and it never does so without explicit
  approval — honoring the Slice 1 "no surprising file mutation" north star.

---

## 5. Triggering & Concurrency

- **Tags:** `theconstruct/summarize`, `theconstruct/tag`, `theconstruct/organize`
  (plus existing `theconstruct/research`). Each maps to a rule in config.
- **One tag per action:** applying several tags to a note enqueues several
  independent runs.
- **Serialize per note:** the per-note guard changes from *skip* to *queue* — if
  a note already has a non-terminal run, a newly-triggered action for that note
  is recorded as `queued` and started when the active run finishes. This prevents
  two in-place edits racing on the same file. Implementation: a per-note async
  lock (keyed by canonical path) in the watch loop, plus the existing DB state
  check. Ordering among queued actions on one note is not guaranteed in v2 (best
  effort, FIFO where simple).

---

## 6. Configuration (TOML) additions

```toml
# existing research agent/rule unchanged …

[[agents]]
name = "Librarian"
domain = "notes"
provider = "ollama"
model = "qwen3:4b-instruct-2507-q8_0"   # local; fast instruct model for structured output
base_url = "http://192.168.1.33:11434"
tools = []                               # these actions need no web tools
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
max_tags = 8          # cap (optional; default 8)

[actions.organize]
exclude_dirs = [".obsidian", ".trash"]   # in addition to dotfolders (optional)
```

Config validation: every `rule.pipeline` must be a known built-in
(`research|summarize|tag|organize`); every `rule.agent` must exist (already
validated in Slice 1).

---

## 7. Components / File Map (informs the plan)

- `construct-config`: add `[actions.*]` structs; extend `validate` for known
  pipelines.
- `construct-obsidian`:
  - `block.rs`: generalize managed-block helpers to take a marker name (research,
    summary) instead of a hardcoded one.
  - `frontmatter.rs`: helpers to read/merge a `tags:` list; set/remove arbitrary
    keys (mostly present).
  - new `vault.rs`: scan vault for folder list and existing tag set (pure-ish I/O,
    unit-testable with a temp dir).
- `construct-engine`:
  - `pipeline.rs` → split into `pipelines/` module: `research.rs`, `summarize.rs`,
    `tag.rs`, `organize.rs`, plus shared `mod.rs` (the `Pipeline` trait + common
    claim/finalize helpers). Pure transforms stay pure + unit-tested.
  - `gate.rs`: add per-action validators (`validate_summary`, `validate_tags`,
    `validate_organize`) alongside the research gate.
  - `orchestrator.rs`: dispatch by pipeline name; generalize reconcile; per-note
    serialization hook.
- `construct-cli`: `init` sample config gains the Librarian agent + 3 rules; new
  `prompts/librarian.md`.

---

## 8. Error Handling

Unchanged Slice 1 guarantees, now per-pipeline: any gate failure, model error, or
guard rejection sets `construct_status: error` with a precise message via the
deterministic `fail()` path; the watcher never crashes on one note. Organize's
folder guard and tag's normalization are deterministic, so a misbehaving model
cannot move a note to an invalid place or inject malformed tags.

---

## 9. Testing Strategy

- **Phase 0:** all existing Slice 1 tests pass unchanged; add dispatch tests
  (a `summarize` tag routes to the summarize pipeline; unknown pipeline → config
  error).
- **Per action (pure transforms, mock model + temp dirs):**
  - Summarize: managed block inserted at top, idempotent re-run, action items
    rendered; empty tldr → error.
  - Tag: merge unions with existing tags, dedupes, normalizes, caps at N; never
    removes existing.
  - Organize: valid destination → proposal + review → accept moves file + records
    `moved_from`; invalid destination → error; reject strips proposal, no move;
    name-collision handling.
- **Serialization:** two actions queued on one note run one-at-a-time, no file
  clobber (full pipeline test with mock model).
- **Vault scan:** folder list + existing-tag gathering over a temp vault.

---

## 10. Success Criteria

- Tagging a note with `#theconstruct/summarize` prepends a correct TL;DR block
  with no review step, on the local model.
- `#theconstruct/tag` adds sensible, normalized tags that reuse existing vault
  tags, without duplicating or removing any.
- `#theconstruct/organize` proposes a destination from real vault folders, and
  only moves the file after the user sets `accepted`; the move is recorded and
  reversible.
- Research (Slice 1) continues to work unchanged.
- Multiple action tags on one note never corrupt the file (serialized).

---

## Appendix A — Future: `claude -p` provider for research

Research is where a frontier model earns its keep (long multi-step tool use,
reliable JSON, convergence) and where the local model is weakest (observed in
Slice 1 live testing: malformed JSON, iteration-budget exhaustion). A future
`ModelProvider` implementation could shell out to the Claude CLI in print mode
(`claude -p "<prompt>"`), parse its output, and satisfy the same `ModelProvider`
trait — so the research pipeline can use a frontier model while the note actions
stay local. This is **documented only**, not built in Slice 2. Open questions for
that future slice: tool-calling via the CLI vs. doing web fetch deterministically
and passing context in; auth/rate/availability; streaming; cost visibility.

## Appendix B — Future: multiple cooperating local models

We have several local models available (`phi4-mini:3.8b`,
`qwen3:4b-instruct-2507-q8_0`, `gemma*`, `qwen3.6:27b`). A future phase could
split a single action across models — e.g. a larger model does the *parsing /
reasoning* over the note, then a small, fast, instruction-tuned model is asked
only to *emit strict JSON* from that reasoning (mitigating the malformed-JSON
failure mode cheaply). This maps cleanly onto the existing per-agent model
config and the `ModelProvider` trait: a pipeline could reference two agents
(a "reasoner" and a "formatter"). **Documented only**, not built in Slice 2;
revisit alongside the JSON-robustness backlog item.
