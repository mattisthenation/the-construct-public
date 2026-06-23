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

/// Starter config with the vault path substituted into the template.
pub fn generated_config(vault_path: &str) -> String {
    // TOML basic-string escaping: a vault dir named e.g. `My "Notes"` must not
    // produce an unparseable config on first run.
    let escaped = vault_path.replace('\\', "\\\\").replace('"', "\\\"");
    crate::commands::SAMPLE_CONFIG.replace(
        "path = \"~/ObsidianVault\"",
        &format!("path = \"{escaped}\""),
    )
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
        let toml = generated_config(&shellexpand(&vault));
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

    println!("\nSetup complete. Next:\n  construct config-check\n  construct watch");
    Ok(())
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
        let toml = generated_config("/Users/example/Vault");
        let cfg: construct_config::Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.vault.path, "/Users/example/Vault");
    }

    #[test]
    fn config_from_template_escapes_toml_special_chars() {
        let toml = generated_config(r#"/Users/m/My "Notes" Vault\x"#);
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
