# The Construct — Product Requirements Document (PRD)

**Date:** 2026-05-29
**Status:** Draft for product validation
**Owner:** Matt Littlehale
**Related:** [Slice 1 Design Spec](2026-05-29-the-construct-slice-1-design.md)

---

## 1. Summary

**The Construct** is a local-first, config-driven agent runtime that runs on the
user's own machine/home network. It connects the user's knowledge tools
(starting with Obsidian) to AI agents — but inverts the usual "AI does
everything" model: a **deterministic outer shell** owns all triggers and side
effects, and a **single agentic step** does only the open-ended reasoning. The
product's first capability turns a tagged Obsidian note into a researched,
reviewable draft without the user leaving their vault.

The north star: *make agentic automation feel trustworthy and boring* — the user
always knows what triggered, what ran, and what changed, and nothing mutates
their files without an explicit, reversible approval step.

---

## 2. Problem & Motivation

Knowledge workers who live in Obsidian (and similar tools) want AI assistance,
but existing options force a bad trade:

- **Cloud AI plugins** send notes to third parties, hide what they do, and act
  with little determinism or auditability.
- **Hand-rolled scripts** are deterministic but brittle and not agentic.
- **General chat assistants** require constant copy-paste and live outside the
  user's knowledge system.

There is no tool that is simultaneously **local-first**, **deterministic and
auditable**, **agentic when it needs to be**, and **native to the user's
existing vault**.

---

## 3. Goals & Non-Goals

### Goals
1. Let the user trigger agent work *declaratively* from inside Obsidian (a tag),
   not by switching tools.
2. Keep every side effect deterministic, observable, and reversible.
3. Run on local models by default; treat frontier models as an opt-in swap.
4. Be config-driven so new agents/tools/rules need no code changes (within the
   provided extension points).
5. Ship a genuinely pleasant CLI/TUI entry experience (`entertheconstruct`).

### Non-Goals (this release)
- Not a multi-user / hosted SaaS.
- Not a general-purpose chatbot replacement.
- Not a full plugin marketplace yet.
- No vector search / vault RAG yet.
- No Notion or Google Drive integration yet.
- No mobile / web client.

---

## 4. Target User

**Primary (v1): the builder-owner — Matt.** A technical Obsidian power user who
runs local models on a home network, values privacy and determinism, and is
comfortable editing a TOML config. Success for v1 is measured against this user.

**Secondary (future): technical Obsidian users** who want local-first AI
automation and can follow a setup guide. The architecture should not foreclose
serving them, but v1 does not optimize for non-technical onboarding.

---

## 5. User Stories (Slice 1)

1. *As the user, I create a note, add `#theconstruct/research`, and the agent
   researches the topic on the web and drops a reviewable draft into the note —
   without me opening another app.*
2. *As the user, I can see exactly what the agent found and where it came from
   (sources), and accept or reject it by changing one frontmatter field.*
3. *As the user, nothing is moved or rewritten until I accept — and rejecting
   cleanly removes the agent's draft.*
4. *As the user, I run `entertheconstruct` and get a live view of what the
   watcher is doing, what runs are active, and a chat pane to talk to my local
   model directly.*
5. *As the user, I can point the system at a different local model, change the
   trigger tag, or change the destination folder by editing config — no
   recompile of behavior I was promised was configurable.*

---

## 6. Functional Requirements

| ID | Requirement | Priority |
| --- | --- | --- |
| FR1 | Watch a configured Obsidian vault for note changes (debounced). | Must |
| FR2 | Recognize a configured trigger tag and route it to a pipeline + named agent. | Must |
| FR3 | Run an agentic web-research loop (local Ollama model + `web_search`, `web_fetch` tools), bounded by iteration/timeout/token budget. | Must |
| FR4 | Validate agent output against a schema **and a source-grounding check** before any write (deterministic gate): every cited `source.url` must appear in the evidence the agent actually gathered (tool outputs + fetched URLs), so invented sources are rejected. | Must |
| FR5 | Write results into a managed, replaceable block in the note; drive a frontmatter status state machine. | Must |
| FR6 | Pause for human accept/reject via frontmatter; finalize deterministically on the decision. | Must |
| FR7 | Persist run + event state in SQLite; resume across restarts. | Must |
| FR8 | Provide `entertheconstruct` CLI with `init`, `config check`, `watch`, `status`, `runs`. | Must |
| FR9 | Provide a styled TUI: live dashboard + interactive chat with the local model. | Must |
| FR10 | Define agents, rules, tools, vault, and model in TOML config. | Must |
| FR11 | Never mutate files from the agent step; only deterministic stages write. | Must |
| FR12 | Surface errors into note frontmatter + event log without crashing the watcher. | Must |
| FR13 | Support a frontier `ModelProvider` and SearXNG search backend *via the trait/abstraction* (impl deferred). | Should |

---

## 7. Success Metrics

Because v1 serves a single owner-user, metrics are usage- and trust-based, not
market metrics:

- **Activation:** the user runs at least one research note end-to-end (trigger →
  accept) within the first session.
- **Trust:** zero instances of an unexpected/unapproved file mutation.
- **Reliability:** the watcher survives a week of normal vault use without a
  crash; failed runs always surface as `status: error`, never silent.
- **Loop quality (directional):** of finalized runs (denominator = runs reaching
  `done` + `rejected`), the share reaching `done` (accepted). Computed directly
  from `run_events` finalize records — no extra instrumentation needed. Treated
  as a *directional* signal, not a hard gate, given the small-N solo usage; a
  sustained low accept rate is the trigger to revisit the model or prompt.
- **Extensibility proof:** adding a second tool or swapping the model requires
  only config + a trait impl, no engine changes.

---

## 8. Constraints & Assumptions

- **Language:** Rust.
- **Primary DB:** SQLite now (must abstract to allow Postgres later).
- **Vector DB:** planned but out of scope for v1.
- **Runtime env:** local machine / home network; Ollama reachable over LAN.
- **Search:** Tavily/Brave API (key via env var) for v1; SearXNG later.
- **Distribution:** single binary, invoked as `entertheconstruct`.
- **Aesthetic:** earthy blue/brown CLI/TUI theme.
- Assumes the user can run Ollama and obtain a search API key.

---

## 9. Risks

| Risk | Mitigation |
| --- | --- |
| Local models are unreliable at multi-step tool calling. | Bound the loop; deterministic gate rejects malformed output; frontier swap available via trait. |
| Local model emits *plausible but fabricated* sources/findings that pass a shape check. | Grounding check (FR4): every `source.url` must be in gathered evidence, else the gate rejects. Findings-grounding is a future tightening. |
| File-watcher feedback loops (engine reacts to its own writes). | Engine-written status values are never decision values; watcher only triggers on absent-status (new) or `accepted`/`rejected` (decision); plus debounce. The user-edit-vs-engine-write distinction rests on these disjoint value sets. |
| Human approval via raw frontmatter YAML editing is itself a UX risk (typos no-op silently; user can break YAML; managed-block hand-edits). | v1 accepts this with a documented contract (§4.2 of design); malformed notes are skipped, not fatal; richer in-app approval is a later slice. Acknowledged limitation. |
| Crash mid-`research` leaves a run stuck and the note silently un-processed. | Startup reconciliation (§4.3 of design) re-triggers runs left in `queued`/`researching`; `review` runs are left parked. |
| Search API (Tavily/Brave) key missing, rate-limited, or down. | Tool returns an error → loop surfaces it → gate fails → `status: error` with message (FR12); never silent. |
| Scope creep into the full platform. | Strict slice boundaries; deferred items enumerated in spec & PRD. |
| TUI complexity balloons. | v1 TUI = dashboard + chat only. |
| Obsidian sync writing partial files. | Treat malformed notes as skippable events, not fatal. |

---

## 10. Future Roadmap (post-v1, indicative)

1. Vault RAG (vector DB + `vault_search` tool).
2. Frontier model provider implementation.
3. User-authored pipelines in TOML.
4. Additional triggers/integrations (Notion, Google Drive).
5. Richer interactive TUI (approve/edit in-app).
6. Postgres store option.

---

## 11. Open Questions

1. ~~Exact local Ollama model + host to default to.~~ **Resolved:**
   `qwen3.6:27b` at `http://192.168.1.33:11434` (swap to
   `qwen3:4b-instruct-2507-q8_0` for speed).
2. Tavily vs Brave as the first search backend (cost/quality).
3. ~~Whether `finalize` should move the note by default or only on opt-in.~~
   **Resolved:** v1 does **not** move the note (honors the "no surprising file
   mutation" north star); `managed_folder` is reserved for a future opt-in move.
4. Whether chat in the TUI should eventually share tools with agents.
