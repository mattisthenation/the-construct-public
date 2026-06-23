mod commands;
mod setup;
mod theme;
mod tui;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load <config-dir>/.env so API keys written by `construct setup` are
    // available without shell-profile exports. Existing process env vars always
    // win (dotenvy never overrides).
    if let Some(env_path) = commands::default_config_dir().map(|d| d.join(".env")) {
        if env_path.exists() {
            let _ = dotenvy::from_path(&env_path);
        }
    }

    // Install a logging subscriber so the watcher's tracing output (run errors,
    // skipped-feature warnings, scheduler/inbox activity) is actually visible.
    // Default level is `info`; override with e.g. `RUST_LOG=construct=debug`.
    // Logs go to stderr so stdout banners/TUI stay clean.
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    commands::run().await
}
