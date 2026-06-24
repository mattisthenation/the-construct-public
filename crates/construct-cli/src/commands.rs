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

# Optional cloud escalation agent. Local Ollama is the default and needs no key;
# point an agent at a cloud provider only if you want it. `api_key_env` is the
# NAME of an env var holding the key — the key is never stored in this file.
#   provider = "anthropic"  (base_url defaults to https://api.anthropic.com)
#   provider = "openai"     (any OpenAI-compatible endpoint; set base_url to use
#                            Groq/Together/OpenRouter/vLLM/etc.)
# [[agents]]
# name = "CloudScout"
# domain = "research"
# provider = "anthropic"
# model = "claude-sonnet-4-6"
# base_url = "https://api.anthropic.com"
# api_key_env = "ANTHROPIC_API_KEY"
# tools = ["web_search", "web_fetch"]

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

# file-this deterministic routing: a note containing any of a rule's keywords is
# filed into `folder` with NO model call (the deterministic-first path). Only notes
# that match no rule escalate to the model. Edit/extend these to fit your vault.
[actions.file_this]
rules = [
    { any_of = ["kubernetes", "k8s", "docker", "terraform"], folder = "Reference" },
    { any_of = ["invoice", "receipt", "budget"], folder = "Finance" },
]

# --- Inbox: ON by default ---
# Drop any note into the Inbox folder; once it's sat untouched for `idle_minutes`,
# The Construct enriches links, summarizes, tags, and files it (or recommends a
# folder for your review). Set `idle_minutes` lower for faster pickup. This uses a
# local model — point `agent` at a running Ollama (see above). To turn it off,
# delete this table.
[inbox]
folder = "Inbox"
idle_minutes = 30
agent = "Librarian"

# --- Other automations — OFF unless their table is present. Uncomment to enable. ---

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

/// Install the tracing subscriber. When `file` is set, logs are appended there
/// (no ANSI) so a live TUI stays clean; otherwise they go to stderr. Default level
/// is `info`; override with `RUST_LOG` (e.g. `RUST_LOG=construct=debug`).
fn init_logging(file: Option<&std::path::Path>) {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let to_file = file.and_then(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    });
    match to_file {
        Some(f) => {
            let _ = fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(move || f.try_clone().expect("clone log file handle"))
                .try_init();
        }
        None => {
            let _ = fmt()
                .with_env_filter(filter)
                .with_writer(std::io::stderr)
                .try_init();
        }
    }
}

/// If the Inbox feature is on but its folder is missing, recreate it — asking
/// first when interactive (and telling the user exactly what will happen), or
/// recreating with a log line when headless (no TTY to prompt).
fn ensure_inbox_folder(cfg: &Config) {
    let Some(inbox) = &cfg.inbox else {
        return;
    };
    let vault = crate::tui::watch_loop::expand_vault_path(&cfg.vault.path);
    let dir = std::path::Path::new(&vault).join(&inbox.folder);
    if dir.is_dir() {
        return;
    }
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        println!(
            "\nThe Inbox is enabled but the folder \"{}/\" doesn't exist in your vault.",
            inbox.folder
        );
        let create = dialoguer::Confirm::new()
            .with_prompt(format!(
                "Create \"{}/\" now so The Construct can watch it?",
                inbox.folder
            ))
            .default(true)
            .interact()
            .unwrap_or(false);
        if create {
            match std::fs::create_dir_all(&dir) {
                Ok(()) => println!("Created {}\n", dir.display()),
                Err(e) => eprintln!("Could not create {}: {e}\n", dir.display()),
            }
        } else {
            println!(
                "Left it alone. The Inbox stays inactive until \"{}/\" exists.\n",
                inbox.folder
            );
        }
    } else {
        // Headless (launchd/service): no TTY to prompt — recreate so the default-on
        // feature works, and record it in the log.
        match std::fs::create_dir_all(&dir) {
            Ok(()) => tracing::info!("re-created missing inbox folder {}", dir.display()),
            Err(e) => tracing::warn!("could not create inbox folder {}: {e}", dir.display()),
        }
    }
}

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

    // Route logs: when a full-screen TUI will own the terminal (the watch
    // dashboard, or the no-subcommand TUI), send tracing to a log file under the
    // config dir so it can't corrupt the rendered UI. Otherwise, stderr.
    let tui_owns_terminal = {
        use std::io::IsTerminal;
        std::io::stdout().is_terminal()
            && matches!(
                &cli.command,
                None | Some(Command::Watch { headless: false })
            )
    };
    let log_file = tui_owns_terminal.then(|| base_dir.join("construct.log"));
    init_logging(log_file.as_deref());

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
            // Inbox is on by default. If its folder is gone, ask before recreating
            // it (interactive), or recreate-with-a-log when headless. Done here,
            // before the dashboard grabs the terminal, so the prompt is visible.
            ensure_inbox_folder(&cfg);
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
                    inbox: cfg
                        .inbox
                        .as_ref()
                        .map(|i| (i.folder.clone(), i.idle_minutes)),
                    log_path: log_file.as_ref().map(|p| p.display().to_string()),
                    config_path: config.clone(),
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
        // Inbox is on by default in the starter config.
        let inbox = cfg.inbox.expect("inbox should be enabled by default");
        assert_eq!(inbox.folder, "Inbox");
        assert_eq!(inbox.agent.as_deref(), Some("Librarian"));
        assert_eq!(cfg.actions.tag.max_tags, 8);
        assert_eq!(
            cfg.actions.organize.exclude_dirs,
            vec![".obsidian", ".trash"]
        );
    }
}
