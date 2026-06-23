//! `construct doctor` — environment preflight. Checks the things that bite a new
//! user: is the config valid, is the vault writable, is each configured Ollama
//! host reachable, are referenced API keys present. Prints a checklist and exits
//! non-zero only on hard failures (bad config / unwritable vault) — an unreachable
//! Ollama is a warning, since the deterministic `remind-me` handler needs no model.

use construct_config::Config;
use std::path::Path;
use std::time::Duration;

/// Run all checks. Returns Ok(true) if all hard checks pass, Ok(false) if a hard
/// check failed (caller exits non-zero).
pub async fn run(config_path: &Path) -> anyhow::Result<bool> {
    println!("The Construct — doctor");
    println!("  config: {}", config_path.display());
    let mut hard_ok = true;

    // 1. Config loads + validates.
    let cfg = match Config::load(config_path) {
        Ok(c) => {
            check(true, &format!("config valid ({} rule(s))", c.rules.len()));
            c
        }
        Err(e) => {
            check(false, &format!("config: {e}"));
            println!("\nFix the config first (try `construct init`), then re-run doctor.");
            return Ok(false);
        }
    };

    // 2. Vault exists + is writable.
    let vault = expand_tilde(&cfg.vault.path);
    let vault_path = Path::new(&vault);
    if !vault_path.is_dir() {
        check(false, &format!("vault not found: {vault}"));
        hard_ok = false;
    } else if let Err(e) = writable(vault_path) {
        check(false, &format!("vault not writable: {e}"));
        hard_ok = false;
    } else {
        check(true, &format!("vault writable: {vault}"));
    }

    // 2b. Inbox folder (on by default). Not a hard failure — `watch` creates it.
    if let Some(inbox) = &cfg.inbox {
        let dir = vault_path.join(&inbox.folder);
        if dir.is_dir() {
            check(
                true,
                &format!(
                    "inbox folder: {}/ (idle {}m)",
                    inbox.folder, inbox.idle_minutes
                ),
            );
        } else {
            println!(
                "  !  inbox folder {}/ missing — `construct watch` will create it",
                inbox.folder
            );
        }
    }

    // 3. Ollama reachability — one check per distinct base_url.
    let mut seen = std::collections::BTreeSet::new();
    for agent in &cfg.agents {
        if !seen.insert(agent.base_url.clone()) {
            continue;
        }
        match host_port(&agent.base_url) {
            Some((host, port)) if reachable(&host, port).await => {
                check(true, &format!("provider reachable: {}", agent.base_url));
            }
            _ => {
                // Warning, not a hard failure.
                println!(
                    "  !  provider unreachable: {} (deterministic handlers still work)",
                    agent.base_url
                );
            }
        }
    }

    // 4. Referenced API keys present (web search).
    if let Some(ws) = &cfg.tools.web_search {
        let present = std::env::var(&ws.api_key_env).is_ok();
        if present {
            check(true, &format!("web_search key set ({})", ws.api_key_env));
        } else {
            println!(
                "  !  web_search key {} not set (only needed for research-this)",
                ws.api_key_env
            );
        }
    }

    println!();
    if hard_ok {
        println!("All required checks passed.");
    } else {
        println!("Some required checks failed (see ✗ above).");
    }
    Ok(hard_ok)
}

fn check(ok: bool, msg: &str) {
    println!("  {} {msg}", if ok { '\u{2713}' } else { '\u{2717}' });
}

/// Can we create (and remove) a temp file in `dir`?
fn writable(dir: &Path) -> std::io::Result<()> {
    let probe = dir.join(".construct-doctor-write-probe");
    std::fs::write(&probe, b"ok")?;
    std::fs::remove_file(&probe)
}

/// Parse "http://host:port" → (host, port). Defaults to port 11434 (Ollama).
fn host_port(base_url: &str) -> Option<(String, u16)> {
    let rest = base_url
        .strip_prefix("http://")
        .or_else(|| base_url.strip_prefix("https://"))
        .unwrap_or(base_url);
    let authority = rest.split('/').next().unwrap_or(rest);
    match authority.rsplit_once(':') {
        Some((h, p)) => Some((h.to_string(), p.parse().ok()?)),
        None => Some((authority.to_string(), 11434)),
    }
}

/// TCP-connect with a short timeout — enough to confirm something is listening.
async fn reachable(host: &str, port: u16) -> bool {
    let addr = format!("{host}:{port}");
    matches!(
        tokio::time::timeout(
            Duration::from_millis(800),
            tokio::net::TcpStream::connect(&addr)
        )
        .await,
        Ok(Ok(_))
    )
}

fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_port() {
        assert_eq!(
            host_port("http://localhost:11434"),
            Some(("localhost".into(), 11434))
        );
        assert_eq!(
            host_port("http://192.168.1.50:11434/"),
            Some(("192.168.1.50".into(), 11434))
        );
        assert_eq!(host_port("http://ollama"), Some(("ollama".into(), 11434)));
    }

    #[tokio::test]
    async fn unreachable_port_is_false() {
        // Port 1 is virtually never listening locally.
        assert!(!reachable("127.0.0.1", 1).await);
    }
}
