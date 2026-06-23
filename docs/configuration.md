# Configuration

The Construct is configured by a single TOML file. Everything that could be a
path, a name, a model, a URL, a pattern, or a prompt is configuration — there are
no hardcoded vault paths or model names in the binary.

## Where config lives

The config file is `config.toml` inside the **config home**, resolved in this
order:

1. `$CONSTRUCT_HOME` — set this to any directory for a portable, single-folder
   install. Everything (config, run DB, prompt overrides, `.env`) lives there.
2. `$XDG_CONFIG_HOME/construct` — standard XDG location.
3. `~/.config/construct` — the default on macOS and Linux.

Override the file for a single command with `--config <path>` (a global flag on
every subcommand). The run database (`construct.db`) and the optional `prompts/`
override directory are resolved **relative to the config file's directory**, so a
stable home gives a stable per-machine install whose state survives upgrades.

Create a starter config with `construct init`, or the interactive
`construct setup` (which also collects the vault path and any API keys).

## Secrets and `.env`

API keys are **never** stored in `config.toml`. The config only names the
*environment variable* that holds each key (`api_key_env`). At startup The
Construct loads a `.env` file from the config home, so keys written by
`construct setup` are available without editing your shell profile. Existing
process environment variables always win over `.env`. `construct setup` writes
`.env` with owner-only (`0600`) permissions.

```sh
# .env in your config home — never committed, never logged
TAVILY_API_KEY=tvly-xxxxxxxxxxxxxxxx
ANTHROPIC_API_KEY=sk-ant-xxxxxxxxxxxx
```

## Annotated example

This is the starter config (`construct init`), annotated. Sections that follow it
cover each table in detail.

```toml
[construct]
name = "The Construct"

[vault]
path = "~/ObsidianVault"      # the watched directory (~ is expanded)
managed_folder = "Construct"  # optional: a subfolder the daemon may write to

# Web search backend — only needed for the research-this handler.
# api_key_env is the NAME of an env var, not the key itself.
[tools.web_search]
backend = "tavily"
api_key_env = "TAVILY_API_KEY"

# An agent is a named (provider, model, base_url) bundle a rule can point at.
[[agents]]
name = "Scout"
domain = "research"
provider = "ollama"
model = "qwen2.5:14b"
base_url = "http://localhost:11434"
tools = ["web_search", "web_fetch"]
system_prompt_file = "prompts/scout.md"

[[agents]]
name = "Librarian"
domain = "notes"
provider = "ollama"
model = "qwen2.5:7b"
base_url = "http://localhost:11434"
tools = []
system_prompt_file = "prompts/librarian.md"

# --- The three handlers: a rule maps a trigger tag to (agent, pipeline) ---

[[rules]]
match_tag = "theconstruct/remind-me"   # fully deterministic; never calls the agent
agent = "Librarian"
pipeline = "remind-me"

[[rules]]
match_tag = "theconstruct/file-this"
agent = "Librarian"
pipeline = "file-this"

[[rules]]
match_tag = "theconstruct/research-this"
agent = "Scout"
pipeline = "research-this"

[actions.tag]
max_tags = 8

[actions.organize]
exclude_dirs = [".obsidian", ".trash"]

# file-this deterministic routing: a note containing any keyword is filed into
# `folder` with NO model call. Only notes matching no rule escalate to a model.
[actions.file_this]
rules = [
  { any_of = ["kubernetes", "k8s", "docker", "terraform"], folder = "Reference" },
  { any_of = ["invoice", "receipt", "budget"], folder = "Finance" },
]
```

## Reference

### `[construct]`

| Key | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | string | yes | Display name shown in the dashboard and `config-check`. |

### `[vault]`

| Key | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `path` | string | yes | — | The watched directory. A leading `~` is expanded to your home. May be a plain folder of `.md` files or an Obsidian vault. |
| `managed_folder` | string | no | — | A vault subfolder the daemon is allowed to write its own notes into. Omit it and the daemon only writes back into the notes it processes. |

### `[[agents]]`

An agent is a reusable named bundle of (provider, model, endpoint) that a rule
points at. Define as many as you like.

| Key | Type | Required | Description |
| --- | --- | --- | --- |
| `name` | string | yes | Unique name; referenced by `rules.agent` and `inbox.agent`. |
| `domain` | string | yes | Free-form label (e.g. `research`, `notes`) for your own clarity. |
| `provider` | string | yes | `ollama` (default/local), `anthropic`, or `openai` (any OpenAI-compatible endpoint). |
| `model` | string | yes | Model identifier. For Ollama, a tag you've pulled (`ollama pull <model>`). |
| `base_url` | string | yes | Provider endpoint. For Ollama, `http://localhost:11434` or a LAN address. |
| `tools` | array of string | no | Tools the agent may call. Known tools: `web_search`, `web_fetch`. Default `[]`. |
| `system_prompt_file` | string | no | Path (relative to the config home) to an editable system-prompt template. |
| `api_key_env` | string | no | **Name** of the env var holding this provider's API key. Required for cloud providers; unused for Ollama. The key is never stored in config. |

> `api_key_env` must be a valid environment-variable name (`[A-Za-z_][A-Za-z0-9_]*`).
> Pasting an actual key here (which usually contains dashes) is rejected with a
> clear error — a common foot-gun the config validator catches for you.

### `[[rules]]`

A rule wires an inline trigger tag to a handler pipeline and the agent it should
use. The Construct watches for notes carrying `#<match_tag>`.

| Key | Type | Required | Description |
| --- | --- | --- | --- |
| `match_tag` | string | yes | The tag (without the leading `#`) that triggers this rule, e.g. `theconstruct/remind-me`. |
| `agent` | string | yes | Name of a defined `[[agents]]`. Validated to exist. |
| `pipeline` | string | yes | A known built-in pipeline (see below). Validated. |

Known pipelines: `remind-me`, `file-this`, `research-this` (the three spec
handlers), plus `summarize`, `tag`, and `organize`. (`research` is an accepted
alias for `research-this`.) `remind-me` is deterministic and never contacts its
agent — it still names one only for config uniformity.

### `[tools.web_search]`

Configures the web-search tool used by `research-this`. Optional — omit it and
research falls back to whatever the model knows without live search.

| Key | Type | Required | Description |
| --- | --- | --- | --- |
| `backend` | string | yes | Search backend, e.g. `tavily`. |
| `api_key_env` | string | yes | **Name** of the env var holding the backend's key (e.g. `TAVILY_API_KEY`). |

### `[actions.tag]`

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `max_tags` | integer | `8` | Maximum number of tags the `tag` pipeline will apply to a note. |

### `[actions.organize]`

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `exclude_dirs` | array of string | `[]` | Folders to ignore when listing candidate destinations / scanning the vault, e.g. `[".obsidian", ".trash"]`. |

### `[actions.file_this]`

Deterministic routing rules for the `file-this` handler. Each rule maps a set of
keywords to a destination folder. The **first** rule with any keyword present in
the note body (case-insensitive substring match) wins — and proposes that folder
with **no model call**. A note matching no rule escalates to the model.

```toml
[actions.file_this]
rules = [
  { any_of = ["kubernetes", "k8s", "docker"], folder = "Reference" },
  { any_of = ["invoice", "receipt"], folder = "Finance" },
]
```

| Field | Type | Description |
| --- | --- | --- |
| `any_of` | array of string | Keywords; a case-insensitive substring match against the note body. |
| `folder` | string | Vault-relative destination folder for matching notes. |

## Automations

### `[inbox]` — auto-process idle notes (ON by default)

The flagship feature, and the one table the starter config ships **enabled**.
Drop any note into the inbox folder; once it's sat untouched for `idle_minutes`,
The Construct enriches links, summarizes, tags, and files it (or recommends a
folder for your review). `construct setup` also creates the inbox folder for you.
To turn it off, delete this table.

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `folder` | string | `"Inbox"` | Vault-relative inbox folder to watch. |
| `idle_minutes` | integer | `30` | Process a note after it's been idle this long. Must be > 0. Lower it for faster pickup. |
| `agent` | string | — | Agent to use; defaults to a summarize/tag agent. Validated to exist if named. Uses a local model, so start Ollama for the inbox to run. |

> The remaining tables below are **off unless present** — add them to opt in.

### `[journal]` — daily journal location

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `folder` | string | `"journal"` | Where daily-summary notes are written (`journal/YYYY/MM/DD.md`). |

### `[schedule]` — daily recap

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `daily_time` | string | `"01:00"` | Local 24-hour `HH:MM` time to generate the daily journal recap (with catch-up if missed). |

### `[briefs]` — fold external daily briefs

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `folder` | string | `"AI/DailyBriefs"` | Folder of externally-written briefs (filenames containing `YYYY-MM-DD`); each is folded into the matching journal day note. Must not be empty. |

## Recipes

### Point at a remote Ollama

There's no rebuild and no special mode — just change `base_url` on the agents to
your inference box:

```toml
[[agents]]
name = "Scout"
provider = "ollama"
model = "qwen2.5:14b"
base_url = "http://192.168.1.50:11434"   # LAN inference host
```

Run `construct doctor` to confirm the host is reachable. (An unreachable Ollama
is only a warning — the deterministic `remind-me` handler still works with no
model at all.)

### Add a cloud escalation agent

Cloud providers are opt-in. Define an agent with `provider = "anthropic"` or
`provider = "openai"`, and name the env var holding the key — never the key
itself:

```toml
[[agents]]
name = "CloudScout"
domain = "research"
provider = "anthropic"          # base_url https://api.anthropic.com
model = "claude-sonnet-4-6"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"
tools = ["web_search", "web_fetch"]

[[rules]]
match_tag = "theconstruct/research-this"
agent = "CloudScout"
pipeline = "research-this"
```

`provider = "openai"` works against **any** OpenAI-compatible endpoint — set
`base_url` to Groq, Together, OpenRouter, a self-hosted vLLM, etc. Put the key in
`.env` (or your shell), under the name you gave `api_key_env`.

### Validate before running

```sh
construct config-check    # parses + validates, prints what's enabled
construct doctor          # also checks vault writability and provider reachability
```
