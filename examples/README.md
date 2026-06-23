# Sample Vault

`sample-vault/` is a small, self-contained Obsidian vault (also just a plain
folder of `.md` files) for demoing The Construct end to end. It ships with the
repo so you can watch all three handlers work without setting up your own notes.

## What's inside

```
sample-vault/
├── .obsidian/          # marks this as an Obsidian vault (optional)
├── Inbox/              # notes waiting to be processed
│   ├── Groceries.md            # remind-me  (deterministic, no model)
│   ├── Domain renewal.md       # remind-me  (deterministic, no model)
│   ├── Kubernetes notes.md     # file-this  (needs Ollama)
│   └── Research WASM components.md  # research-this (needs Ollama)
├── Projects/           # a destination folder for file-this
└── Reference/          # a destination folder for file-this
```

Each note is tagged with an inline trigger tag that tells The Construct which
handler to route it through:

| Tag                         | Handler         | Uses a model?                  |
| --------------------------- | --------------- | ------------------------------ |
| `#theconstruct/remind-me`   | `remind-me`     | **No** — fully deterministic   |
| `#theconstruct/file-this`   | `file-this`     | Yes (local Ollama)             |
| `#theconstruct/research-this` | `research-this` | Yes (local Ollama + web search) |

## Try it

1. **Create a config** (writes `~/.config/construct/config.toml`):

   ```sh
   construct init
   ```

2. **Point it at this vault.** Set `vault.path` in the config to the *absolute*
   path of this sample vault, e.g.:

   ```toml
   [vault]
   path = "/absolute/path/to/theconstruct-public/examples/sample-vault"
   ```

3. **Process a single note.** Start with the headline feature — `remind-me`
   runs with **zero model calls**, so you don't need Ollama (or any network) for
   this one:

   ```sh
   construct run "examples/sample-vault/Inbox/Groceries.md"
   ```

   The Construct parses "remind me to buy oat milk tomorrow at 5pm", schedules
   the reminder, and writes the result back into the note — no LLM involved.

4. **Run the live daemon** to process everything as it lands:

   ```sh
   construct watch
   ```

## Ollama

- `remind-me` needs **nothing** — no Ollama, no network. That's the point.
- `file-this` and `research-this` escalate to a model, so they need a local
  [Ollama](https://ollama.com) instance running (`http://localhost:11434` by
  default). Run `construct doctor` to check that Ollama is reachable before
  trying those two.

## Resetting the demo

Processing a note edits it in place and may move `file-this` notes into
`Projects/` or `Reference/`. To start fresh, restore the vault with
`git checkout -- examples/sample-vault` (or `git stash`).
