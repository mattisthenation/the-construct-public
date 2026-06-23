//! Live dashboard for `entertheconstruct watch`: status header, activity feed,
//! pending-review count. Pure consumer of the EngineEvent broadcast — engine
//! pipelines never block on the UI.
use construct_core::store::{RunRecord, Store};
use construct_engine::events::{EngineEvent, EventKind};
use construct_store::SqliteStore;
use crossterm::event::{self, Event, KeyCode};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// Format a run as a single dashboard row. Pure → testable.
// Exercised by tests; not yet wired into the live dashboard render.
#[allow(dead_code)]
pub fn run_row(r: &RunRecord) -> String {
    let name = std::path::Path::new(&r.note_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&r.note_path);
    format!("{:<10} {:<8} {}", r.status.as_str(), &r.agent, name)
}

pub struct DashboardCtx {
    pub vault_path: String,
    pub day_note_path: std::path::PathBuf,
    pub daily_time: Option<String>,
    pub briefs_folder: Option<String>,
    pub db_url: String,
}

struct State {
    activity: VecDeque<(EventKind, String, String)>, // kind, time, message
    pending_review: usize,
    started: Instant,
    paused: bool,
}

pub async fn run_dashboard(
    ctx: DashboardCtx,
    mut rx: broadcast::Receiver<EngineEvent>,
    paused: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let store = SqliteStore::connect(&ctx.db_url).await?;
    let mut terminal = ratatui::init();
    let mut state = State {
        activity: VecDeque::with_capacity(200),
        pending_review: 0,
        started: Instant::now(),
        paused: false,
    };
    let mut last_refresh = Instant::now() - Duration::from_secs(10);

    let res: anyhow::Result<()> = loop {
        // Drain any pending engine events (non-blocking).
        loop {
            match rx.try_recv() {
                Ok(ev) => {
                    if state.activity.len() >= 200 {
                        state.activity.pop_back();
                    }
                    state.activity.push_front((ev.kind, ev.time, ev.message));
                }
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
        // Refresh pending-review count every 5s.
        if last_refresh.elapsed() >= Duration::from_secs(5) {
            if let Ok(runs) = store.list_runs(500).await {
                state.pending_review = runs
                    .iter()
                    .filter(|r| r.status.as_str() == "review")
                    .count();
            }
            last_refresh = Instant::now();
        }

        if let Err(e) = terminal.draw(|f| draw(f, &ctx, &state)) {
            break Err(e.into());
        }

        let polled = match event::poll(Duration::from_millis(200)) {
            Ok(p) => p,
            Err(e) => break Err(e.into()),
        };
        if polled {
            match event::read() {
                Ok(Event::Key(key)) => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                    KeyCode::Char('p') => {
                        state.paused = !state.paused;
                        paused.store(state.paused, Ordering::Relaxed);
                    }
                    KeyCode::Char('o') => {
                        let _ = std::process::Command::new("open")
                            .arg(&ctx.day_note_path)
                            .spawn();
                    }
                    _ => {}
                },
                Ok(_) => {}
                Err(e) => break Err(e.into()),
            }
        }
    };
    ratatui::restore();
    res
}

fn draw(f: &mut Frame, ctx: &DashboardCtx, state: &State) {
    use crate::theme::Theme;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header
            Constraint::Length(1), // status line
            Constraint::Min(3),    // activity
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    let up = state.started.elapsed().as_secs();
    let header = Paragraph::new(format!(
        " The Construct v{}  ·  vault: {}  ·  up {}h {:02}m",
        env!("CARGO_PKG_VERSION"),
        ctx.vault_path,
        up / 3600,
        (up % 3600) / 60,
    ))
    .style(Theme::header());
    f.render_widget(header, chunks[0]);

    let watching = if state.paused {
        "|| paused"
    } else {
        "* watching"
    };
    let daily = ctx
        .daily_time
        .as_deref()
        .map(|t| format!("  daily {t}"))
        .unwrap_or_default();
    let briefs = ctx
        .briefs_folder
        .as_deref()
        .map(|b| format!("  briefs {b}/"))
        .unwrap_or_default();
    let status = Paragraph::new(format!(
        " {watching}{daily}{briefs}  {} pending review",
        state.pending_review
    ))
    .style(Theme::body());
    f.render_widget(status, chunks[1]);

    let items: Vec<ListItem> = state
        .activity
        .iter()
        .map(|(kind, time, msg)| {
            let style = match kind {
                EventKind::Error => Style::default().fg(Color::Red),
                EventKind::Info => Style::default().fg(Color::DarkGray),
                _ => Theme::body(),
            };
            ListItem::new(format!("{time}  {:<6} {msg}", kind.label())).style(style)
        })
        .collect();
    let feed = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Activity ")
            .border_style(Theme::accent()),
    );
    f.render_widget(feed, chunks[2]);

    let footer = Paragraph::new(" q quit  p pause  o open today's note")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, chunks[3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use construct_core::types::{RunId, RunStatus};

    #[test]
    fn formats_run_row_with_basename() {
        let r = RunRecord {
            id: RunId("x".into()),
            rule: "research".into(),
            agent: "Scout".into(),
            note_path: "/vault/My Topic.md".into(),
            status: RunStatus::Review,
            error: None,
        };
        let row = run_row(&r);
        assert!(row.contains("review"));
        assert!(row.contains("Scout"));
        assert!(row.contains("My Topic.md"));
        assert!(!row.contains("/vault/"));
    }
}
