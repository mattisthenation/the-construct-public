use clap::{Parser, Subcommand};
use construct_config::Config;
use construct_core::store::Store;
use construct_store::SqliteStore;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "construct",
    version,
    about = "The Construct — local-first agent runtime"
)]
struct Cli {
    /// Path to config file. Defaults to $CONSTRUCT_HOME/config.toml, else the
    /// XDG location ~/.config/construct/config.toml — so it works from any directory.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

/// The directory that holds `config.toml`, the run DB, prompt overrides, and `.env`.
/// Resolution order: `$CONSTRUCT_HOME` (portable single-folder mode) → XDG
/// `$XDG_CONFIG_HOME/construct` → `~/.config/construct`. Everything resolves
/// relative to this dir, so a stable home gives a stable per-machine install.
pub fn default_config_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CONSTRUCT_HOME") {
        return Some(PathBuf::from(home));
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("construct"));
        }
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("construct"))
}

/// Resolve the config file: explicit `--config` wins, else `<config-dir>/config.toml`,
/// else a last-resort CWD fallback.
fn resolve_config_path(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    default_config_dir()
        .map(|d| d.join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("config.toml"))
}

#[derive(Subcommand)]
enum Command {
    /// Interactive first-run setup: vault path, starter config, API keys.
    Setup {
        /// Run without prompts (requires --vault on first run).
        #[arg(long)]
        non_interactive: bool,
        /// Vault path (non-interactive first run).
        #[arg(long)]
        vault: Option<String>,
        /// KEY=VALUE pairs to store in .env (repeatable).
        #[arg(long = "key")]
        keys: Vec<String>,
    },
    /// Write a starter config.toml.
    Init,
    /// Validate the config file.
    ConfigCheck,
    /// Check the environment: config valid, vault writable, provider reachable.
    Doctor,
    /// Process a single note once, then exit (for testing/scripting).
    Run {
        /// Path to the markdown note to process.
        note: PathBuf,
    },
    /// Run the vault watcher.
    Watch {
        /// Plain log output (no dashboard) — for launchd/background use.
        #[arg(long)]
        headless: bool,
    },
    /// Show watcher/run status summary.
    Status,
    /// List recent runs.
    Runs,
}

pub const SAMPLE_CONFIG: &str = r#"[construct]
name = "The Construct"

[vault]
path = "~/ObsidianVault"
managed_folder = "Construct"

# Web search backend (only needed for the `research-this` handler).
# IMPORTANT: api_key_env is the NAME of an environment variable, NOT the key itself.
# Set the key in your shell:  export TAVILY_API_KEY=tvly-xxxxxxxx
[tools.web_search]
backend = "tavily"
api_key_env = "TAVILY_API_KEY"

# Change `model` to a tag you've pulled (`ollama pull <model>`), and `base_url` to
# wherever Ollama runs (localhost, or a LAN IP like http://192.168.1.50:11434).
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

# --- The three handlers ---

# remind-me: FULLY DETERMINISTIC. Parses "remind me to X [when]" and records it.
# No model is ever called — this handler proves the deterministic-first thesis.
# (It still names an agent for uniformity, but never contacts it.)
[[rules]]
match_tag = "theconstruct/remind-me"
agent = "Librarian"
pipeline = "remind-me"

# file-this: classify and propose a destination folder (routing, reviewed by you).
[[rules]]
match_tag = "theconstruct/file-this"
agent = "Librarian"
pipeline = "file-this"

# research-this: escalate to a model (+ web search) and write a report back.
[[rules]]
match_tag = "theconstruct/research-this"
agent = "Scout"
pipeline = "research-this"

[actions.tag]
max_tags = 8

[actions.organize]
exclude_dirs = [".obsidian", ".trash"]

# --- Automations (Slice 3) — all OFF unless their table is present. Uncomment to enable. ---

# Auto-process top-level notes dropped in the Inbox folder once they've been idle.
# [inbox]
# folder = "Inbox"
# idle_minutes = 30        # process a note after it's been untouched this long
# agent = "Librarian"      # optional; defaults to a summarize/tag agent

# Where the daily-summary journal notes are written (journal/YYYY/MM/DD.md).
# [journal]
# folder = "journal"

# Generate a daily journal recap at this local time (with catch-up if missed).
# [schedule]
# daily_time = "01:00"

# Fold externally-written Daily Briefs (filenames containing YYYY-MM-DD) into
# the matching journal day note, and feed them to the daily recap.
# [briefs]
# folder = "AI/DailyBriefs"
"#;

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = resolve_config_path(cli.config);
    // Resolve paths (the run DB) relative to the config file's directory, so the same
    // DB is used no matter which directory the command is invoked from.
    let base_dir: PathBuf = match config.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let db_url = crate::tui::watch_loop::resolve_db_url(&base_dir);
    match cli.command {
        Some(Command::Setup {
            non_interactive,
            vault,
            keys,
        }) => {
            crate::setup::run_setup(
                &config,
                &base_dir,
                crate::setup::SetupArgs {
                    non_interactive,
                    vault,
                    keys,
                },
            )
            .await?;
        }
        Some(Command::Init) => {
            if config.exists() {
                println!("{} already exists; not overwriting.", config.display());
            } else {
                if let Some(parent) = config.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::write(&config, SAMPLE_CONFIG)?;
                println!("Wrote starter config to {}", config.display());
            }
        }
        Some(Command::ConfigCheck) => {
            let cfg = Config::load(&config)?;
            println!(
                "OK: '{}' with {} agent(s), {} rule(s).",
                cfg.construct.name,
                cfg.agents.len(),
                cfg.rules.len()
            );
            // Surface the Slice 3 automations so a correct config is confirmed wired.
            match &cfg.inbox {
                Some(ib) => println!(
                    "  inbox:    on (folder={}, idle={}m)",
                    ib.folder, ib.idle_minutes
                ),
                None => println!("  inbox:    off"),
            }
            match &cfg.schedule {
                Some(sc) => println!("  schedule: on (daily {})", sc.daily_time),
                None => println!("  schedule: off"),
            }
            match &cfg.briefs {
                Some(b) => println!("  briefs:   on (folder={})", b.folder),
                None => println!("  briefs:   off"),
            }
            // web_search needs an env var; flag if it's set as a name vs. resolves.
            if let Some(ws) = &cfg.tools.web_search {
                let resolved = std::env::var(&ws.api_key_env).is_ok();
                println!(
                    "  web_search: {} → {}",
                    ws.api_key_env,
                    if resolved {
                        "set"
                    } else {
                        "MISSING (export it)"
                    }
                );
            }
        }
        Some(Command::Doctor) => {
            let ok = crate::doctor::run(&config).await?;
            if !ok {
                std::process::exit(1);
            }
        }
        Some(Command::Run { note }) => {
            let cfg = Config::load(&config)?;
            crate::tui::watch_loop::run_once(cfg, base_dir, note).await?;
        }
        Some(Command::Watch { headless }) => {
            use std::io::IsTerminal;
            let cfg = Config::load(&config)?;
            let (events, rx) = construct_engine::events::channel();
            let paused = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let use_dashboard = !headless && std::io::stdout().is_terminal();
            if use_dashboard {
                let today = chrono::Local::now().date_naive();
                let journal_folder = cfg
                    .journal
                    .as_ref()
                    .map(|j| j.folder.clone())
                    .unwrap_or_else(|| "journal".to_string());
                let vault = crate::tui::watch_loop::expand_vault_path(&cfg.vault.path);
                let ctx = crate::tui::dashboard::DashboardCtx {
                    vault_path: vault.clone(),
                    day_note_path: std::path::PathBuf::from(&vault).join(
                        construct_engine::pipelines::daily::journal_day_path(
                            &journal_folder,
                            today,
                        ),
                    ),
                    daily_time: cfg.schedule.as_ref().map(|s| s.daily_time.clone()),
                    briefs_folder: cfg.briefs.as_ref().map(|b| b.folder.clone()),
                    db_url: db_url.clone(),
                };
                // Engine in a background task; dashboard owns the terminal.
                // If the dashboard exits (q / Esc / error), the engine task is
                // aborted and the process exits. If the engine dies, the dashboard
                // keeps rendering (the activity feed simply stops updating) until
                // the user quits.
                let engine = tokio::spawn(crate::tui::watch_loop::run_watch(
                    cfg,
                    base_dir,
                    events,
                    paused.clone(),
                    true, // quiet=true: dashboard owns the terminal, suppress banner println!s
                ));
                let ui = crate::tui::dashboard::run_dashboard(ctx, rx, paused).await;
                engine.abort();
                ui?;
            } else {
                crate::tui::watch_loop::run_watch(cfg, base_dir, events, paused, false).await?;
            }
        }
        Some(Command::Runs) => {
            let store = SqliteStore::connect(&db_url).await?;
            let runs = store.list_runs(20).await?;
            if runs.is_empty() {
                println!("No runs yet.");
            }
            for r in runs {
                println!("{}  {:<10}  {}", r.id, r.status.as_str(), r.note_path);
            }
        }
        Some(Command::Status) => {
            let store = SqliteStore::connect(&db_url).await?;
            let runs = store.list_runs(1000).await?;
            if runs.is_empty() {
                println!("No runs yet.");
            } else {
                use std::collections::BTreeMap;
                let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
                for r in &runs {
                    *counts.entry(r.status.as_str()).or_default() += 1;
                }
                println!("{} run(s):", runs.len());
                for (status, n) in counts {
                    println!("  {status:<11} {n}");
                }
                let in_review: Vec<_> = runs
                    .iter()
                    .filter(|r| r.status.as_str() == "review")
                    .collect();
                if !in_review.is_empty() {
                    println!("awaiting review:");
                    for r in in_review {
                        println!("  {}", r.note_path);
                    }
                }
            }
        }
        None => {
            // No subcommand → launch the TUI.
            let cfg = Config::load(&config).ok();
            crate::tui::run_tui(cfg).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_config_is_valid() {
        let cfg: Config = toml::from_str(SAMPLE_CONFIG).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.agents.len(), 2);
        assert_eq!(cfg.rules.len(), 3);
        assert_eq!(cfg.agents[0].name, "Scout");
        assert_eq!(cfg.agents[1].name, "Librarian");
        // The three spec handlers are all present and route to known pipelines.
        let pipelines: Vec<&str> = cfg.rules.iter().map(|r| r.pipeline.as_str()).collect();
        assert!(pipelines.contains(&"remind-me"));
        assert!(pipelines.contains(&"file-this"));
        assert!(pipelines.contains(&"research-this"));
        assert_eq!(cfg.actions.tag.max_tags, 8);
        assert_eq!(
            cfg.actions.organize.exclude_dirs,
            vec![".obsidian", ".trash"]
        );
    }
}
