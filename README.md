# The Construct

**The folder is the prompt.** Drop a markdown note into a watched directory; The
Construct reads it, decides how to handle it, does the work, writes the result
back into your notes, and logs it. It's a local-first companion for your Obsidian
vault (or any plain folder of `.md` files) — not a plugin, a guest that runs
beside it.

```sh
brew install construct                       # see docs/install.md

echo "Remind me to call the dentist tomorrow at 5pm #theconstruct/remind-me" > vault/note.md
construct run vault/note.md
```

The note now reads:

```markdown
---
construct_reminder: call the dentist
construct_reminder_due: 2026-06-24T17:00:00-07:00
construct_status: done
---
<!-- construct:reminder:start -->
**⏰ Reminder:** call the dentist
- **Due:** 2026-06-24 17:00 (Wed)
- **Captured:** 2026-06-23
- _Handled deterministically — no model call._
<!-- construct:reminder:end -->
```

No LLM ran. No network call. No tokens spent. That is the whole point.

## The thesis

> **Most of your agent calls didn't need to be model calls.**

The Construct is **deterministic-first**. When a note arrives, the system tries
to handle it with rules, parsing, and plain code *before* it ever considers an
LLM. A model call is the most expensive, slowest, least reliable option — so it's
the last resort, reserved for the steps that genuinely need reasoning.

Internally there are two components, and they're the spine of how the system reads:

- **Priori** — the judgment layer. It reads an incoming note and the pipeline its
  tag selected, then decides: *handle this deterministically, or escalate to a
  model?* This is the gate.
- **Determa** — the execution engine. It runs the deterministic code paths once
  Priori clears a note for handling without a model.

```
note → Priori (judge) → Determa (deterministic execution)
                      ↘ escalate to a model (last resort) → write result back → log
```

Local-first, bring-your-own-everything. No telemetry, no phone-home, no hosted
dependency. Point it at a local [Ollama](https://ollama.com) and the entire tool
runs with **zero cloud calls**. The cloud-call boundary is opt-in and auditable.

## The three handlers

A note is routed by an inline tag. The defaults:

| Tag | Handler | Uses a model? |
| --- | --- | --- |
| `#theconstruct/remind-me` | **remind-me** | **No** — fully deterministic |
| `#theconstruct/file-this` | **file-this** | Only if no keyword rule matches |
| `#theconstruct/research-this` | **research-this** | Yes — escalates to a model |

- **remind-me** parses `remind me to <task> [<when>]` (`tomorrow`, `tonight`,
  `in 3 days`, `next friday`, an ISO date, `at 5pm`, …), writes a reminder block
  and `construct_reminder` / `construct_reminder_due` frontmatter, and marks the
  note done. It **never touches a model** — it works with Ollama off and the
  network unplugged. This is the handler that makes the thesis checkable.
- **file-this** routes a note to a folder. Deterministic first: the
  `[actions.file_this]` rules map keywords to folders, and a match proposes a move
  with **no model call**. Only a miss escalates to a small local classifier.
  Moves are proposed for your review, never silently applied.
- **research-this** escalates to a model (plus web search, if configured), then
  writes a structured, source-grounded report back into the note for review.

## Quickstart

```sh
construct setup          # interactive: vault path, starter config, API keys
construct config-check   # validate the config
construct doctor         # check the environment (config, vault, Ollama)
construct watch          # run the live dashboard daemon
```

`construct watch` opens a TUI dashboard: a live activity feed (deterministic
handling shows in distinct green — "no model call"), recently processed notes,
and a status panel for daemon health, provider reachability, and queue depth.
For backgrounding under launchd, run it headless:

```sh
construct watch --headless
```

Want to try it without your own notes? A demo vault ships in
[`examples/sample-vault/`](examples/sample-vault) with a note for each handler —
see [`examples/README.md`](examples/README.md).

### Commands

| Command | What it does |
| --- | --- |
| `construct setup` | First-run setup: vault path, starter config, API keys |
| `construct init` | Write a starter `config.toml` |
| `construct config-check` | Validate the config and report what's enabled |
| `construct doctor` | Check config, vault writability, provider reachability |
| `construct run <note>` | Process a single note once, then exit |
| `construct watch [--headless]` | Run the watcher daemon (TUI, or plain logs) |
| `construct status` | Run counts and notes awaiting review |
| `construct runs` | List recent runs |

## Providers

[Ollama](https://ollama.com) is the default and first-class citizen — local,
free, on-thesis. Point an agent at `http://localhost:11434` (or a LAN box) and
nothing leaves your machine. Cloud escalation is opt-in per agent: set
`provider = "anthropic"` or `provider = "openai"` (any OpenAI-compatible
endpoint — Groq, Together, OpenRouter, vLLM, …) and name the env var holding the
key with `api_key_env`. The key itself is never stored in config. See
[`docs/configuration.md`](docs/configuration.md).

## Configuration

Everything is config-driven — paths, folder names, models, base URLs, trigger
patterns, prompts. Config lives at `~/.config/construct/config.toml` (XDG-aware;
override the whole home with `$CONSTRUCT_HOME` for a portable single-folder
install, or a single run with `--config`). Secrets go in a `.env` next to the
config, loaded at startup. Full reference: [`docs/configuration.md`](docs/configuration.md).

## Documentation

- [Installation](docs/install.md) — Homebrew, build from source, release process
- [Configuration](docs/configuration.md) — every key, defaults, remote Ollama, cloud agents
- [Handlers](docs/handlers.md) — how routing works and how to author a new handler
- [Security](docs/security.md) — threat model, vault integrity, network egress
- [Sample vault](examples/README.md) — try all three handlers end to end

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option — the conventional permissive pairing for Rust projects, so the tool
drops cleanly into anyone's stack.
