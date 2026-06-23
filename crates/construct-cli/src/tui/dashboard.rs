//! Live dashboard for `construct watch` — a terminal "ops console" with a hacker
//! vibe (neon-green-on-black, digital rain) that still reads as a real status
//! surface. Pure consumer of the EngineEvent broadcast — engine pipelines never
//! block on the UI.
//!
//! Layout: a title bar, then a left column of two stacked panels (Activity over
//! Recent Notes) beside a right column of four stacked boxes (logo, rain,
//! commands, status). The daemon also runs headless; this is only a view.
use crate::tui::rain::Rain;
use construct_core::store::{RunRecord, Store};
use construct_engine::events::{EngineEvent, EventKind};
use construct_store::SqliteStore;
use crossterm::event::{self, Event, KeyCode};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, Paragraph};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

// --- Hacker palette: neon green + cyan on the terminal's own black. -----------
const NEON: Color = Color::Rgb(0, 255, 150); // brightest — headlines, the head of a glyph
const GREEN: Color = Color::Rgb(0, 200, 90); // primary text/accents
const DIM: Color = Color::Rgb(0, 90, 60); // borders, labels, tails
const CYAN: Color = Color::Rgb(90, 220, 220); // keys, secondary accents
const AMBER: Color = Color::Rgb(255, 190, 70); // pending / paused / review
const RED: Color = Color::Rgb(255, 80, 80); // errors
const FG: Color = Color::Rgb(170, 225, 195); // body, greenish off-white

/// ASCII/text logo for the dashboard. Intentionally tiny.
// design-todo: replace with final ASCII logo (see docs/design-todo.md).
const LOGO: &str = "╔═╗
║C║  THE CONSTRUCT
╚═╝  the folder is the prompt";

/// A dim-green panel with a bracketed, cyan, uppercased title — the console motif.
fn panel(title: &str, bright: bool) -> Block<'_> {
    let border = if bright { GREEN } else { DIM };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ))
}

/// Format a run as a single "Recent Notes" row: note basename + status.
/// Pure → testable.
pub fn run_row(r: &RunRecord) -> String {
    let name = std::path::Path::new(&r.note_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&r.note_path);
    format!("{:<8} {}", r.status.as_str(), name)
}

/// A small status glyph for a run outcome — purely cosmetic.
fn status_glyph(status: &str) -> char {
    match status {
        "done" | "accepted" => '✓',
        "review" => '◆',
        "error" | "rejected" => '✗',
        "running" | "researching" | "queued" => '▸',
        _ => '·',
    }
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
    processed: u64,                                  // lifetime events seen (for the footer pulse)
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
        processed: 0,
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
                    state.processed = state.processed.saturating_add(1);
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
    // Title bar on top; everything else is a left/right split below it.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(f.area());

    draw_title(f, ctx, state, rows[0]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(rows[1]);

    draw_left(f, cols[0], state); // two stacked panels: Activity + Recent Notes
    draw_right(f, ctx, state, cols[1]); // four stacked boxes: logo, rain, keys, status
}

fn draw_title(f: &mut Frame, _ctx: &DashboardCtx, state: &State, area: Rect) {
    let (dot, dot_color, word) = if state.paused {
        ('⏸', AMBER, "paused")
    } else {
        ('●', NEON, "watching")
    };
    let up = state.started.elapsed().as_secs();
    let line = Line::from(vec![
        Span::styled(
            " THE CONSTRUCT ",
            Style::default().fg(NEON).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(DIM),
        ),
        Span::styled("  deterministic-first daemon  ", Style::default().fg(GREEN)),
        Span::styled(format!("{dot} "), Style::default().fg(dot_color)),
        Span::styled(
            word,
            Style::default().fg(dot_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  up {}h{:02}m", up / 3600, (up % 3600) / 60),
            Style::default().fg(DIM),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

/// Left column: Activity (top) over Recent Notes (bottom), stacked.
fn draw_left(f: &mut Frame, area: Rect, state: &State) {
    let stack = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(area);

    // --- Activity (live event log) ---
    let items: Vec<ListItem> = state
        .activity
        .iter()
        .map(|(kind, time, msg)| {
            let (label_color, msg_color) = match kind {
                EventKind::Error => (RED, RED),
                EventKind::Info => (DIM, DIM),
                // The thesis, made visible: "handled without a model" glows neon.
                EventKind::Deterministic => (NEON, NEON),
                _ => (CYAN, FG),
            };
            let bold = if *kind == EventKind::Deterministic {
                Modifier::BOLD
            } else {
                Modifier::empty()
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{time} "), Style::default().fg(DIM)),
                Span::styled(
                    format!("{:<8}", format!("[{}]", kind.label())),
                    Style::default().fg(label_color),
                ),
                Span::styled(
                    format!(" {msg}"),
                    Style::default().fg(msg_color).add_modifier(bold),
                ),
            ]))
        })
        .collect();
    f.render_widget(List::new(items).block(panel("ACTIVITY", true)), stack[0]);

    // --- Recent Notes (note basename + outcome/status) ---
    let recent: Vec<ListItem> = state
        .recent
        .iter()
        .map(|r| {
            let s = r.status.as_str();
            let color = match s {
                "error" | "rejected" => RED,
                "review" => AMBER,
                "done" | "accepted" => GREEN,
                _ => FG,
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", status_glyph(s)), Style::default().fg(color)),
                Span::styled(run_row(r), Style::default().fg(color)),
            ]))
        })
        .collect();
    f.render_widget(
        List::new(recent).block(panel("RECENT NOTES", false)),
        stack[1],
    );
}

/// Right column: Logo, Digital Rain, Commands, Status — four boxes stacked.
fn draw_right(f: &mut Frame, ctx: &DashboardCtx, state: &mut State, area: Rect) {
    let boxes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // logo
            Constraint::Min(4),    // rain (flexes to fill)
            Constraint::Length(5), // commands
            Constraint::Length(9), // status (fits dot + up/queue/events/vault + daily/briefs)
        ])
        .split(area);

    // --- Logo ---
    let logo = Paragraph::new(LOGO)
        .style(Style::default().fg(NEON).add_modifier(Modifier::BOLD))
        .block(panel("◈", false));
    f.render_widget(logo, boxes[0]);

    // --- Digital rain ---
    draw_rain(f, boxes[1], &mut state.rain);

    // --- Commands ---
    let key = |k: &str, label: &str| {
        Line::from(vec![
            Span::styled(format!(" {k} "), Style::default().fg(Color::Black).bg(CYAN)),
            Span::styled(format!("  {label}"), Style::default().fg(FG)),
        ])
    };
    let commands = Paragraph::new(vec![
        key("q", "quit"),
        key("p", "pause / resume"),
        key("o", "open today's note"),
    ])
    .block(panel("KEYS", false));
    f.render_widget(commands, boxes[2]);

    // --- Status ---
    f.render_widget(status_widget(ctx, state), boxes[3]);
}

fn status_widget<'a>(ctx: &'a DashboardCtx, state: &State) -> Paragraph<'a> {
    let (dot, dot_color, word) = if state.paused {
        ('⏸', AMBER, "paused")
    } else {
        ('●', NEON, "watching")
    };
    let up = state.started.elapsed().as_secs();
    let pending_color = if state.pending_review > 0 { AMBER } else { DIM };

    let row = |label: &'static str, value: String, color: Color| {
        Line::from(vec![
            Span::styled(format!(" {label:<8}"), Style::default().fg(DIM)),
            Span::styled(value, Style::default().fg(color)),
        ])
    };

    let vault = std::path::Path::new(&ctx.vault_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&ctx.vault_path)
        .to_string();

    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!(" {dot} "), Style::default().fg(dot_color)),
            Span::styled(
                word,
                Style::default().fg(dot_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        row(
            "uptime",
            format!("{}h {:02}m", up / 3600, (up % 3600) / 60),
            FG,
        ),
        row(
            "queue",
            format!("{} review", state.pending_review),
            pending_color,
        ),
        row("events", format!("{} processed", state.processed), FG),
        row("vault", vault, FG),
    ];
    if let Some(t) = &ctx.daily_time {
        lines.push(row("daily", t.clone(), FG));
    }
    if let Some(b) = &ctx.briefs_folder {
        lines.push(row("briefs", format!("{b}/"), FG));
    }
    Paragraph::new(lines).block(panel("STATUS", true))
}

/// Render the digital-rain panel. CPU-safe: we advance the animation exactly
/// once per draw (the dashboard render loop is throttled to ~5fps by its 200ms
/// event poll), and `Rain::step`/`Rain::cell` are O(area) integer math with no
/// threads, timers, or per-cell syscalls.
fn draw_rain(f: &mut Frame, area: Rect, rain: &mut Rain) {
    let block = panel("RAIN", false);
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
                        0 => NEON,  // head: brightest
                        1 => GREEN, // body
                        _ => DIM,   // tail: dim
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

    #[test]
    fn status_glyphs_cover_outcomes() {
        assert_eq!(status_glyph("done"), '✓');
        assert_eq!(status_glyph("review"), '◆');
        assert_eq!(status_glyph("error"), '✗');
        assert_eq!(status_glyph("queued"), '▸');
        assert_eq!(status_glyph("mystery"), '·');
    }
}
