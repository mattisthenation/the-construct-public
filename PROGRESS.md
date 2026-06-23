# The Construct — Build Progress

> Running memory across loop iterations. Trust this over in-context recollection;
> reconcile against actual repo state at the start of each loop. See `docs/loop-log.md`
> for the dated narrative of what happened each iteration.

## Decisions (from Matt, 2026-06-23)
- **Refactor the existing personal-Construct crates in place** (don't clean-room rebuild).
- **CLAUDE.md is the source of truth** when it conflicts with existing code (naming, handlers).
- **Run fully autonomous** to the 8-point Definition of Done.

## Current state snapshot (updated each loop)
- Workspace builds (`cargo build` green). Reconstructed the missing root `Cargo.toml`
  workspace manifest (all crates referenced `.workspace = true` but the root was never copied in).
- **184 tests pass**, `cargo fmt --check` clean, `cargo clippy --all-targets -D warnings` clean.
- 8 crates: `construct-{cli,core,config,engine,store,obsidian,tools,model-ollama}`.

## Gap analysis vs CLAUDE.md spec (the work list)
Ordered roughly by value toward Definition of Done.

| # | Item | Status |
|---|------|--------|
| 1 | Workspace builds + green baseline | ✅ done |
| 2 | Rename binary `entertheconstruct` → `construct` | ✅ done |
| 3 | Config path → XDG `~/.config/construct/config.toml` (+ env override) | ✅ done |
| 4 | Add `construct run <note>` subcommand | ✅ done |
| 5 | Add `construct doctor` subcommand | ✅ done |
| 6 | **`remind-me` handler — fully deterministic, zero model calls (thesis proof)** | ✅ done |
| 7 | `file-this` deterministic-first (rules before any model) | ✅ done (keyword rules → folder, zero model; escalates on miss) |
| 8 | Internal module naming: Priori (judge) / Determa (execute) | ✅ done (priori::judge seam; Determa = deterministic pipelines) |
| 9 | Cloud providers: Anthropic + OpenAI-compatible (escalation tier) | ✅ done |
| 10 | TUI: recent-notes pane, matrix-rain panel, logo placeholder | ✅ done |
| 11 | `examples/sample-vault/` with demo notes for all 3 handlers | ✅ done |
| 12 | Homebrew tap + formula, Apple Silicon + Intel builds | ✅ done (formula + release.yml; sha filled at first real release) |
| 13 | GitHub Actions CI: fmt, clippy -D warnings, test, cargo audit, release build | ✅ done |
| 14 | `cargo audit` clean (or findings documented) | ✅ done (0 vulns; 2 transitive warnings to document in security.md) |
| 15 | `docs/security.md` security audit | ⬜ todo |
| 16 | Docs: 30s README, config reference, handler-authoring guide, `docs/design-todo.md` | ⬜ todo |

### Notes that map to spec handlers
- `research-this` ← `pipelines/research.rs` (exists, matches spec — escalates + web tools + grounding gate).
- `file-this` ← `pipelines/{organize,tag,inbox}.rs` (exists but escalate-only; needs deterministic rule tier).
- `remind-me` ← **does not exist**; must build. Highest narrative value.
- Other existing pipelines (brief, daily, journal_tag, summarize) are extra; keep but not in the launch-bar 3.

## Already-good (don't rebuild)
- `ModelProvider` trait in `construct-core` is pluggable; Ollama is the default impl.
- `construct-config` is TOML-driven; vault/model/url/prompts already externalized (path resolution needs XDG fix).
- `gate.rs` does model-output grounding validation (rejects fabricated sources) — keep.
- SSRF guard in `construct-tools/web_fetch.rs` blocks private IP ranges — keep (those "192.168" hits are correct test cases, not personal data).

## Needs Matt's decision (route around, don't block)
- (none yet)

## Deferred / parked
- `scripts/{setup-home,update,release}.sh` still reference the old `entertheconstruct`
  binary and `~/.theconstruct` single-folder dist model. These get rewritten or removed
  wholesale when we build the Homebrew install flow (item 12), not patched piecemeal.

## Definition of Done checklist
1. ⬜ Tests/clippy/fmt/audit green  2. ⬜ 3 handlers demoable + remind-me zero-model
3. ⬜ Homebrew-installable + doctor passes  4. ⬜ Fully configurable (remote Ollama via config only)
5. ⬜ Daemon robust (malformed notes, outages, pathological input)  6. ⬜ TUI matches mockup + headless
7. ⬜ Security audit + docs/security.md  8. ⬜ Public-ready docs + clean history
