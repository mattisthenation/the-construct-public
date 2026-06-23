mod commands;
mod doctor;
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

    // Logging is installed inside `commands::run` once the subcommand is known —
    // when a full-screen TUI will own the terminal, logs are routed to a file so
    // they can't corrupt the dashboard; otherwise they go to stderr.
    commands::run().await
}
