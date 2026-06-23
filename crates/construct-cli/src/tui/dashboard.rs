//! Live dashboard for `construct watch`: status header, activity feed,
//! pending-review count. Pure consumer of the EngineEvent broadcast — engine
//! pipelines never block on the UI.
use crate::tui::rain::Rain;
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

/// ASCII/text logo for the dashboard. Intentionally tiny.
// design-todo: replace with final ASCII logo (see docs/design-todo.md).
const LOGO: &str = "  ╔═╗
  ║C║  THE
  ╚═╝  CONSTRUCT";

/// Format a run as a single "Recent Notes" row: note basename + status.
/// Pure → testable.
pub fn run_row(r: &RunRecord) -> String {
    let name = std::path::Path::new(&r.note_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&r.note_path);
    format!("{:<8} {}", r.status.as_str(), name)
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
    recent: Vec<RunRecord>,                          // recently processed notes
    pending_review: usize,
    started: Instant,
    paused: bool,
    rain: Rain,
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
        recent: Vec::new(),
        pending_review: 0,
        started: Instant::now(),
        paused: false,
        rain: Rain::new(),
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
        // Refresh pending-review count + recent notes every 5s (same cadence).
        if last_refresh.elapsed() >= Duration::from_secs(5) {
            if let Ok(runs) = store.list_runs(500).await {
                state.pending_review = runs
                    .iter()
                    .filter(|r| r.status.as_str() == "review")
                    .count();
                // Most-recent first (list_runs returns newest first); cap for display.
                state.recent = runs.into_iter().take(50).collect();
            }
            last_refresh = Instant::now();
        }

        if let Err(e) = terminal.draw(|f| draw(f, &ctx, &mut state)) {
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

fn draw(f: &mut Frame, ctx: &DashboardCtx, state: &mut State) {
    // Vertical: title bar, top region (two panes), bottom row (four boxes), footer.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title bar
            Constraint::Min(5),    // top region (Activity | Recent Notes)
            Constraint::Length(8), // bottom row (Logo | Rain | Commands | Status)
            Constraint::Length(1), // footer keybindings
        ])
        .split(f.area());

    draw_title(f, ctx, rows[0]);
    draw_top(f, rows[1], state);
    draw_bottom(f, ctx, state, rows[2]);

    let footer = Paragraph::new(" q quit  p pause  o open today's note")
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, rows[3]);
}

fn draw_title(f: &mut Frame, ctx: &DashboardCtx, area: Rect) {
    use crate::theme::Theme;
    let title = Paragraph::new(format!(
        " The Construct  ·  v{}  ·  vault: {}",
        env!("CARGO_PKG_VERSION"),
        ctx.vault_path,
    ))
    .style(Theme::header());
    f.render_widget(title, area);
}

/// Top region: Activity (left) and Recent Notes (right), side by side.
fn draw_top(f: &mut Frame, area: Rect, state: &State) {
    use crate::theme::Theme;
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // --- Activity (live event log) ---
    let items: Vec<ListItem> = state
        .activity
        .iter()
        .map(|(kind, time, msg)| {
            let style = match kind {
                EventKind::Error => Style::default().fg(Color::Red),
                EventKind::Info => Style::default().fg(Color::DarkGray),
                // The thesis, made visible: "handled without a model" glows green.
                EventKind::Deterministic => Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
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
    f.render_widget(feed, panes[0]);

    // --- Recent Notes (note basename + outcome/status) ---
    let recent: Vec<ListItem> = state
        .recent
        .iter()
        .map(|r| {
            let style = match r.status.as_str() {
                "error" => Style::default().fg(Color::Red),
                "review" => Style::default().fg(Color::Yellow),
                "done" | "accepted" => Style::default().fg(Color::Green),
                _ => Theme::body(),
            };
            ListItem::new(run_row(r)).style(style)
        })
        .collect();
    let recent_list = List::new(recent).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Recent Notes ")
            .border_style(Theme::accent()),
    );
    f.render_widget(recent_list, panes[1]);
}

/// Bottom row: Logo, Digital Rain, Commands, Status — four boxes side by side.
fn draw_bottom(f: &mut Frame, ctx: &DashboardCtx, state: &mut State, area: Rect) {
    use crate::theme::Theme;
    let boxes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(22), // logo
            Constraint::Percentage(28), // rain
            Constraint::Percentage(22), // commands
            Constraint::Percentage(28), // status
        ])
        .split(area);

    // --- Logo ---
    let logo = Paragraph::new(LOGO).style(Theme::accent()).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" The Construct ")
            .border_style(Theme::accent()),
    );
    f.render_widget(logo, boxes[0]);

    // --- Digital rain ---
    draw_rain(f, boxes[1], &mut state.rain);

    // --- Commands ---
    let commands = Paragraph::new(" q  quit\n p  pause\n o  open today's note")
        .style(Theme::body())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Commands ")
                .border_style(Theme::accent()),
        );
    f.render_widget(commands, boxes[2]);

    // --- Status ---
    let up = state.started.elapsed().as_secs();
    let watching = if state.paused {
        "|| paused"
    } else {
        "* watching"
    };
    let daily = ctx
        .daily_time
        .as_deref()
        .map(|t| format!("\n daily   {t}"))
        .unwrap_or_default();
    let briefs = ctx
        .briefs_folder
        .as_deref()
        .map(|b| format!("\n briefs  {b}/"))
        .unwrap_or_default();
    let status = Paragraph::new(format!(
        " {watching}\n up      {}h {:02}m\n pending {} review{daily}{briefs}",
        up / 3600,
        (up % 3600) / 60,
        state.pending_review,
    ))
    .style(Theme::body())
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Status ")
            .border_style(Theme::accent()),
    );
    f.render_widget(status, boxes[3]);
}

/// Render the digital-rain panel. CPU-safe: we advance the animation exactly
/// once per draw (the dashboard render loop is throttled to ~5fps by its 200ms
/// event poll), and `Rain::step`/`Rain::cell` are O(area) integer math with no
/// threads, timers, or per-cell syscalls.
fn draw_rain(f: &mut Frame, area: Rect, rain: &mut Rain) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Rain ")
        .border_style(Style::default().fg(Color::Green));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    rain.step(inner.width, inner.height);

    let mut lines: Vec<Line> = Vec::with_capacity(inner.height as usize);
    for row in 0..inner.height {
        let mut spans: Vec<Span> = Vec::with_capacity(inner.width as usize);
        for col in 0..inner.width {
            match rain.cell(col, row) {
                Some((g, tier)) => {
                    let color = match tier {
                        0 => Color::Rgb(180, 255, 180), // head: bright
                        1 => Color::Green,
                        _ => Color::Rgb(0, 90, 0), // tail: dim
                    };
                    spans.push(Span::styled(g.to_string(), Style::default().fg(color)));
                }
                None => spans.push(Span::raw(" ")),
            }
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), inner);
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
        assert!(row.contains("My Topic.md"));
        assert!(!row.contains("/vault/"));
    }
}
