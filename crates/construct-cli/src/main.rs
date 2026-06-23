mod commands;
mod setup;
mod theme;
mod tui;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load ~/.theconstruct/.env (or $CONSTRUCT_HOME/.env) so API keys written
    // by `entertheconstruct setup` are available without shell-profile exports.
    // Existing process env vars always win (dotenvy never overrides).
    if let Some(env_path) = construct_home_env_path() {
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

/// $CONSTRUCT_HOME/.env, else ~/.theconstruct/.env. Mirrors the config-path
/// resolution in commands.rs.
fn construct_home_env_path() -> Option<std::path::PathBuf> {
    if let Some(home) = std::env::var_os("CONSTRUCT_HOME") {
        return Some(std::path::PathBuf::from(home).join(".env"));
    }
    std::env::var_os("HOME").map(|h| {
        std::path::PathBuf::from(h)
            .join(".theconstruct")
            .join(".env")
    })
}
