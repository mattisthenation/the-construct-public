# The Construct — Slice 2 Backlog

Captured during Slice 1 live smoke testing (2026-05-30). These are deferred
improvements, not Slice 1 scope.

## 1. Panic hardening at the tool/handler boundary
**Why:** During live testing, a panic inside `web_fetch` (UTF-8 bug, since
fixed in `91a3719`) escaped the deterministic `fail()` path because it happened
inside a `tokio::spawn`ed task in the watch loop. The watcher survived, but the
run died *silently* — the note never reached `construct_status: error`.

**Fix:** Wrap each spawned event handler so a panic still routes the note to
`status: error` instead of vanishing. Options:
- Catch the `JoinError` from the spawned task (`handle.await` returns
  `Err(JoinError)` with `.is_panic()`), and on panic run the deterministic
  `fail()` for that run.
- Or `std::panic::catch_unwind` at the `Tool::call` boundary, converting a
  panic into `ToolError::Failed(...)` so the existing error path handles it.
- Prefer the JoinError approach — keeps tools simple and covers panics anywhere
  in the handler, not just in tools.

**Test:** a tool/mock that panics → assert the note ends in `construct_status:
error` and a `run_event` records it, and the watcher keeps running.

## 2. Live "found a file" indication (watch mode UX)
**Why:** In `watch` mode it's hard to tell what's happening — the user could
see it "found and started" but not much else. Local models are slow, so silent
multi-minute work is confusing.

**Fix:** Emit clear, themed progress lines (or feed the TUI dashboard) as a run
moves through stages, e.g.:
- `→ detected #theconstruct/research in <note> (run <id>)`
- `→ researching… (model qwen3.6:27b, iteration N/max)`
- `→ gate: ok / rejected (<reason>)`
- `→ wrote draft, awaiting your review (set construct_status: accepted/rejected)`
- `→ finalized: done / rejected`

Drive these off the `run_events` the orchestrator already writes, so the watch
CLI and the TUI dashboard share one source of truth. Consider a `--verbose`
flag and/or a live tail of `run_events`.

## 3. Robust JSON extraction + gate resilience for local models
**Why (OBSERVED):** First real run on the journal note failed with
`agent output was not valid JSON: expected ',' or '}' at line 2 column 496`.
The local model (qwen3.6:27b) produced **malformed/truncated JSON** — the most
common real failure mode, more so than fabricated sources. The gate's
`extract_json` does a single balanced-brace scan and `serde_json::from_str`;
any stray comma, comment, trailing text, or truncation kills the whole run.

**Fix (graduated, keep determinism + no-fabrication guarantee):**
- **Coax cleaner output:** strengthen `prompts/scout.md` to demand a single
  fenced ```json block and nothing else; consider Ollama's structured-output /
  JSON mode (response_format / format=json) so the model is constrained to valid
  JSON at generation time. This is the highest-leverage fix.
- **Repair loop:** on gate failure, feed the exact parser error back to the
  model for ONE bounded retry ("your JSON was invalid here: …; return only
  valid JSON"). Cheapest robust win; reuses the existing loop budget.
- **Tolerant extraction:** try the fenced block first; on failure, attempt a
  lenient parse (e.g. strip trailing commas / comments) before giving up.
- **Token budget:** "column 496 line 2" suggests possible truncation — make
  sure max-tokens for the final answer is generous enough for the JSON.

**Test:** model emits JSON with a trailing comma / wrapped in prose / truncated
→ repair retry recovers it; genuinely unrecoverable → `error` with the parser
message (current behavior preserved).

## 3b. Agent loop termination + iteration budget (OBSERVED)
**Why:** Run `efdbfc10` (Deterministic First.md) failed with
`exceeded max iterations (8)`. The web-research loop is capped at 8 iterations
(hardcoded in `watch_loop.rs` `max_iterations: 8`). Real research burns
iterations fast (1 search + several fetches + reasoning), AND local models
sometimes keep calling tools without ever emitting a final plain-text answer.
Both hit the ceiling.

**Fix (combine):**
- **Make budget configurable** per-agent in `construct.toml` (e.g.
  `max_iterations`) instead of hardcoded; raise the default (12–16).
- **Force a final answer on the last iteration:** when one iteration remains,
  re-prompt with tools disabled ("you must now answer in JSON; no more tools"),
  so a budget exhaustion becomes a real answer attempt the gate can judge,
  rather than a hard error.
- **Detect no-progress loops:** if the model repeats the same tool call with
  identical args, short-circuit toward the final-answer prompt.
- Pairs naturally with #3 (robust JSON) and #2 (progress UX shows iteration N).

**Test:** model that always calls tools → on the final iteration it's forced to
answer (tools removed) and the gate evaluates that answer; a model that answers
early still terminates normally.

## 4. Grounding-gate leniency (secondary; not yet observed failing)
Once JSON parses reliably, revisit the source-grounding check: dropping
ungrounded sources (keeping grounded ones, fail only if zero remain) and
normalizing URLs (trailing slash, http/https, www) before the substring match
may be gentler than hard-fail. NOTE: not yet observed as a real failure — the
first run died on JSON, not grounding. Validate the need before building.

---

### Already done in Slice 1 (for reference)
- UTF-8-safe `web_fetch` (`91a3719`) — was panicking on multi-byte chars.
- Deterministic source-grounding gate, frontmatter contract, crash
  reconciliation, no-note-move-by-default (all in the Slice 1 design/plan).
