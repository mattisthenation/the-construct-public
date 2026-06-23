//! Interactive first-run setup: vault path, starter config, API keys → .env.
//! Design rules: never rewrite an existing construct.toml; never echo a key;
//! .env is always chmod 600. `--non-interactive` makes this scriptable.
use anyhow::Context;
use std::path::{Path, PathBuf};

/// One promptable API key.
pub struct KeySpec {
    pub env: &'static str,
    pub label: &'static str,
}

/// Keys worth prompting for: anything the loaded config references via
/// api_key_env, plus the well-known set for features the user may enable
/// later.
pub fn key_specs(cfg: Option<&construct_config::Config>) -> Vec<KeySpec> {
    let mut keys = vec![KeySpec {
        env: "TAVILY_API_KEY",
        label: "Tavily (web search)",
    }];
    if let Some(cfg) = cfg {
        if let Some(ws) = &cfg.tools.web_search {
            if !keys.iter().any(|k| k.env == ws.api_key_env) {
                // Config names a custom env var: prompt for that instead.
                keys.insert(
                    0,
                    KeySpec {
                        env: Box::leak(ws.api_key_env.clone().into_boxed_str()),
                        label: "web search",
                    },
                );
            }
        }
    }
    keys
}

/// Pure: add or replace KEY=VALUE in .env content, preserving everything else.
pub fn upsert_env(existing: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let mut found = false;
    let mut lines: Vec<String> = existing
        .lines()
        .map(|l| {
            if l.starts_with(&prefix) {
                found = true;
                format!("{key}={value}")
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(format!("{key}={value}"));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// TOML basic-string escaping: a value like `My "Notes"` must not produce an
/// unparseable config.
fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Starter config with the vault path and inbox folder substituted into the template.
pub fn generated_config(vault_path: &str, inbox_folder: &str) -> String {
    crate::commands::SAMPLE_CONFIG
        .replace(
            "path = \"~/ObsidianVault\"",
            &format!("path = \"{}\"", toml_escape(vault_path)),
        )
        // The only `folder = "Inbox"` in the template is the [inbox] one.
        .replace(
            "folder = \"Inbox\"",
            &format!("folder = \"{}\"", toml_escape(inbox_folder)),
        )
}

/// Editable prompt templates, embedded so they can be deployed regardless of how
/// the binary was installed. Users edit the copies in their config dir.
const PROMPT_SCOUT: &str = include_str!("../../../prompts/scout.md");
const PROMPT_LIBRARIAN: &str = include_str!("../../../prompts/librarian.md");
const PROMPT_DAILY: &str = include_str!("../../../prompts/daily_summary.md");

/// Write the prompt templates into `<home>/prompts/` (the config dir) so the
/// configured `system_prompt_file` paths resolve. Never clobbers an edited file.
fn deploy_prompts(home: &Path) -> anyhow::Result<()> {
    let dir = home.join("prompts");
    std::fs::create_dir_all(&dir)?;
    let mut wrote = 0;
    for (name, body) in [
        ("scout.md", PROMPT_SCOUT),
        ("librarian.md", PROMPT_LIBRARIAN),
        ("daily_summary.md", PROMPT_DAILY),
    ] {
        let p = dir.join(name);
        if !p.exists() {
            std::fs::write(&p, body)?;
            wrote += 1;
        }
    }
    if wrote > 0 {
        println!("  prompts: {wrote} template(s) → prompts/");
    }
    Ok(())
}

/// Write .env with owner-only permissions. Refuses to leave it looser.
pub fn write_env_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // mode(0o600) applies at creation, so secrets never exist at umask perms;
    // the set_permissions re-assert tightens a pre-existing looser file.
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    f.write_all(contents.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("chmod 600 {}", path.display()))?;
    Ok(())
}

/// Looks like an Obsidian vault (or at least an existing directory we can use).
fn validate_vault_path(input: &str) -> Result<(), String> {
    let p = PathBuf::from(shellexpand(input));
    if !p.is_dir() {
        return Err(format!("{} is not a directory", p.display()));
    }
    if !p.join(".obsidian").is_dir() {
        return Err(format!(
            "{} exists but has no .obsidian/ — enter it again to use anyway",
            p.display()
        ));
    }
    Ok(())
}

fn shellexpand(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    p.to_string()
}

pub struct SetupArgs {
    pub non_interactive: bool,
    pub vault: Option<String>,
    /// KEY=VALUE pairs.
    pub keys: Vec<String>,
}

/// Entry point. `config_path` is the resolved construct.toml path; `home` its dir.
pub async fn run_setup(config_path: &Path, home: &Path, args: SetupArgs) -> anyhow::Result<()> {
    println!("The Construct — setup");
    println!("  home:   {}", home.display());

    // --- construct.toml ---
    let existing_cfg = if config_path.exists() {
        let cfg =
            construct_config::Config::load(config_path).map_err(|e| anyhow::anyhow!("{}", e))?;
        println!(
            "  config: {} (existing — left untouched)",
            config_path.display()
        );
        Some(cfg)
    } else {
        let vault = match (&args.vault, args.non_interactive) {
            (Some(v), _) => v.clone(),
            (None, true) => anyhow::bail!("--non-interactive requires --vault on first run"),
            (None, false) => prompt_vault_path()?,
        };
        let inbox_folder = if args.non_interactive {
            "Inbox".to_string()
        } else {
            prompt_inbox_folder()?
        };
        let toml = generated_config(&shellexpand(&vault), &inbox_folder);
        std::fs::create_dir_all(home)?;
        std::fs::write(config_path, &toml)?;
        println!("  config: {} (created)", config_path.display());
        Some(construct_config::Config::load(config_path).map_err(|e| anyhow::anyhow!("{}", e))?)
    };

    // --- API keys → .env ---
    let env_path = home.join(".env");
    let mut env_text = std::fs::read_to_string(&env_path).unwrap_or_default();
    if args.non_interactive {
        for pair in &args.keys {
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--key expects KEY=VALUE, got '{pair}'"))?;
            env_text = upsert_env(&env_text, k.trim(), v.trim());
        }
    } else {
        for spec in key_specs(existing_cfg.as_ref()) {
            let already = std::env::var(spec.env).is_ok()
                || env_text
                    .lines()
                    .any(|l| l.starts_with(&format!("{}=", spec.env)));
            let label = if already {
                format!(
                    "{} [{}] — already set; enter to keep, or paste a new key",
                    spec.label, spec.env
                )
            } else {
                format!(
                    "{} [{}] — paste key, or enter to skip",
                    spec.label, spec.env
                )
            };
            let value: String = dialoguer::Password::new()
                .with_prompt(label)
                .allow_empty_password(true)
                .interact()?;
            if !value.trim().is_empty() {
                env_text = upsert_env(&env_text, spec.env, value.trim());
            }
        }
    }
    if !env_text.is_empty() {
        write_env_file(&env_path, &env_text)?;
        println!("  keys:   {} (mode 600)", env_path.display());
    }

    // --- Deploy editable prompt templates next to the config ---
    if let Err(e) = deploy_prompts(home) {
        eprintln!("  \u{26a0} could not deploy prompts: {e}");
    }

    // --- Seed the vault: create the Inbox folder + drop a readme guide ---
    if let Some(cfg) = &existing_cfg {
        if let Err(e) = seed_vault(cfg) {
            // Non-fatal: a bad vault path shouldn't abort an otherwise-good setup.
            eprintln!("  \u{26a0} could not seed vault: {e}");
        }
    }

    println!(
        "\nSetup complete. Next:\n  construct config-check\n  construct watch\n\nOpen \"the-construct-readme.md\" in your vault for a quick tour."
    );
    Ok(())
}

/// Ensure the Inbox folder exists and drop a `the-construct-readme.md` guide into
/// the vault root. Best-effort and non-destructive: the readme is written only if
/// absent (a user's edits are never clobbered). The vault is sacred.
fn seed_vault(cfg: &construct_config::Config) -> anyhow::Result<()> {
    let vault = PathBuf::from(shellexpand(&cfg.vault.path));
    if !vault.is_dir() {
        anyhow::bail!("vault path {} is not a directory", vault.display());
    }
    // Inbox is on by default — make sure the folder exists so drop-in works now.
    if let Some(inbox) = &cfg.inbox {
        std::fs::create_dir_all(vault.join(&inbox.folder))?;
        println!("  inbox:  {}/ (ready)", inbox.folder);
    }
    let readme = vault.join("the-construct-readme.md");
    if readme.exists() {
        println!("  readme: the-construct-readme.md (exists — left untouched)");
    } else {
        std::fs::write(&readme, readme_markdown(cfg))?;
        println!("  readme: the-construct-readme.md (created)");
    }
    Ok(())
}

/// The in-vault user guide. IMPORTANT: every example trigger tag is wrapped in
/// `inline code` so it is NOT a bare whitespace-delimited `#tag` token — otherwise
/// the watcher would treat this readme as a note to process. (The watcher detects
/// tags via `Note::tags()`, which only matches tokens that *start* with `#`.)
fn readme_markdown(cfg: &construct_config::Config) -> String {
    let tag_for = |p: &str| {
        cfg.rules
            .iter()
            .find(|r| r.pipeline == p)
            .map(|r| r.match_tag.clone())
    };
    let remind = tag_for("remind-me").unwrap_or_else(|| "theconstruct/remind-me".into());
    let file = tag_for("file-this").unwrap_or_else(|| "theconstruct/file-this".into());
    let research = tag_for("research-this").unwrap_or_else(|| "theconstruct/research-this".into());
    let inbox_folder = cfg
        .inbox
        .as_ref()
        .map(|i| i.folder.clone())
        .unwrap_or_else(|| "Inbox".into());
    let idle = cfg.inbox.as_ref().map(|i| i.idle_minutes).unwrap_or(30);

    format!(
        r#"# The Construct — how to use this vault

The Construct is running beside this vault as a quiet companion. **The folder is
the prompt:** you drop a markdown note, it reads it, does the work, and writes the
result back — handling most things with plain code and only reaching for a model
when it truly needs to.

> You can delete this note anytime — it's just a guide. The Construct won't
> re-create it unless you run `construct setup` again.

## The Inbox (the easy way)

Drop any note into the **`{inbox_folder}/`** folder. After it's sat untouched for
about {idle} minutes, The Construct enriches any links, summarizes it, tags it, and
either files it into a fitting folder or suggests one for your review. Want faster
pickup? Lower `idle_minutes` in your config.

## Tag a note (the explicit way)

Add one of these tags anywhere in a note's body and The Construct handles it:

| Put this tag in a note | What happens |
| --- | --- |
| `#{remind}` | **Reminder.** Parses "remind me to X by/at/on …" and records it. **No model — instant, works offline.** |
| `#{file}` | **File it.** Routes the note to a folder (keyword rules first; a local model only if needed). Proposed for your review. |
| `#{research}` | **Research it.** Uses a local model (+ web search if configured) to write a sourced report back into the note. |

### Try it now

Create a note that says **"Remind me to call the dentist tomorrow at 5pm"** and
tag it with `#{remind}` (type the tag yourself — it's shown in code here so this
guide doesn't get processed as a reminder). Within a moment The Construct rewrites
the note with a tidy reminder block and a due date — and tells you it did it
*without calling a model*.

## Watching it work

- `construct watch` opens a live dashboard (Activity, Recent Notes, status).
  Deterministic, no-model handling shows in bright green — that's the whole idea.
- `construct status` prints run counts and anything awaiting your review.
- `construct doctor` checks your setup (config, vault, whether Ollama is reachable).

## Good to know

- **`#{remind}` needs nothing else.** `#{file}` and `#{research}` use a local model
  via [Ollama](https://ollama.com) — start it (`ollama serve`) for those to run.
- The Construct only ever proposes folder moves for your review; it won't shuffle
  your vault behind your back.
- Settings live in your config file (`construct config-check` shows the path). The
  Inbox folder, the idle delay, the tags, and the models are all yours to change.

Happy building.
"#
    )
}

fn prompt_vault_path() -> anyhow::Result<String> {
    use dialoguer::Input;
    loop {
        let input: String = Input::new()
            .with_prompt("Path to your Obsidian vault")
            .interact_text()?;
        match validate_vault_path(&input) {
            Ok(()) => return Ok(input),
            Err(msg) => {
                eprintln!("  \u{26a0} {msg}");
                // Second consecutive identical answer = use anyway.
                let again: String = Input::new()
                    .with_prompt("Path to your Obsidian vault (repeat to confirm)")
                    .interact_text()?;
                if again == input {
                    return Ok(input);
                }
            }
        }
    }
}

/// Ask which folder to use as the Inbox (drop-zone for auto-processed notes).
/// Defaults to "Inbox". Empty input keeps the default.
fn prompt_inbox_folder() -> anyhow::Result<String> {
    use dialoguer::Input;
    let folder: String = Input::new()
        .with_prompt("Inbox folder — drop notes here to auto-process them")
        .default("Inbox".to_string())
        .interact_text()?;
    let f = folder.trim();
    Ok(if f.is_empty() {
        "Inbox".to_string()
    } else {
        f.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_env_adds_updates_and_preserves() {
        let out = upsert_env("", "TAVILY_API_KEY", "tvly-1");
        assert_eq!(out, "TAVILY_API_KEY=tvly-1\n");
        let out = upsert_env(&out, "OTHER", "x");
        let out = upsert_env(&out, "TAVILY_API_KEY", "tvly-2"); // replace in place
        assert_eq!(out.matches("TAVILY_API_KEY").count(), 1);
        assert!(out.contains("TAVILY_API_KEY=tvly-2"));
        assert!(out.contains("OTHER=x"));
        // Comments and unknown lines survive.
        let cur = "# my keys\nFOO=bar\n";
        let out = upsert_env(cur, "BAZ", "qux");
        assert!(out.starts_with("# my keys\nFOO=bar\n"));
        assert!(out.contains("BAZ=qux"));
    }

    #[test]
    fn config_from_template_substitutes_vault_path() {
        let toml = generated_config("/Users/example/Vault", "Inbox");
        let cfg: construct_config::Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.vault.path, "/Users/example/Vault");
        assert_eq!(cfg.inbox.unwrap().folder, "Inbox");
    }

    #[test]
    fn config_from_template_substitutes_inbox_folder() {
        let toml = generated_config("/v", "📥 Capture");
        let cfg: construct_config::Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.inbox.unwrap().folder, "📥 Capture");
    }

    #[test]
    fn config_from_template_escapes_toml_special_chars() {
        let toml = generated_config(r#"/Users/m/My "Notes" Vault\x"#, "Inbox");
        let cfg: construct_config::Config = toml::from_str(&toml).unwrap();
        assert_eq!(cfg.vault.path, r#"/Users/m/My "Notes" Vault\x"#);
    }

    #[test]
    fn key_specs_collects_configured_env_names() {
        let cfg: construct_config::Config = toml::from_str(crate::commands::SAMPLE_CONFIG).unwrap();
        let keys = key_specs(Some(&cfg));
        assert!(keys.iter().any(|k| k.env == "TAVILY_API_KEY"));
        // No config yet → still suggests the known key set.
        assert!(key_specs(None).iter().any(|k| k.env == "TAVILY_API_KEY"));
    }

    #[test]
    fn readme_has_no_live_trigger_tags() {
        // The readme must never contain a bare `#tag` token, or the watcher would
        // try to process the readme itself. Parse it the way the watcher does.
        let cfg: construct_config::Config = toml::from_str(crate::commands::SAMPLE_CONFIG).unwrap();
        let md = readme_markdown(&cfg);
        let note = construct_obsidian::frontmatter::Note::parse(&md);
        assert!(
            note.tags().is_empty(),
            "readme leaked live trigger tags: {:?}",
            note.tags()
        );
        // Sanity: it does still mention the tags (inside backticks).
        assert!(md.contains("remind-me"));
    }

    #[tokio::test]
    async fn non_interactive_setup_creates_config_and_env_600() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("Vault");
        std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
        let home = tmp.path().join("home");
        let config = home.join("construct.toml");

        run_setup(
            &config,
            &home,
            SetupArgs {
                non_interactive: true,
                vault: Some(vault.to_string_lossy().to_string()),
                keys: vec!["TAVILY_API_KEY=tvly-test".into()],
            },
        )
        .await
        .unwrap();

        let cfg = construct_config::Config::load(&config).unwrap();
        assert_eq!(cfg.vault.path, vault.to_string_lossy());
        let env_path = home.join(".env");
        assert!(std::fs::read_to_string(&env_path)
            .unwrap()
            .contains("TAVILY_API_KEY=tvly-test"));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&env_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        // Re-run: config untouched, key replaceable.
        run_setup(
            &config,
            &home,
            SetupArgs {
                non_interactive: true,
                vault: None,
                keys: vec!["TAVILY_API_KEY=tvly-2".into()],
            },
        )
        .await
        .unwrap();
        let env = std::fs::read_to_string(&env_path).unwrap();
        assert!(env.contains("tvly-2"));
        assert_eq!(env.matches("TAVILY_API_KEY").count(), 1);
    }
}
