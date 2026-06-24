//! Live dashboard for `construct watch` — a terminal "ops console" with a hacker
//! vibe (neon-green-on-black) that still reads as a real status surface. Pure
//! consumer of the EngineEvent broadcast — engine pipelines never block on the UI.
//!
//! Layout: a title bar, then a left column of two stacked panels (Activity over
//! Recent Notes) beside a right column of four stacked boxes (logo, the
//! deterministic-first meter, commands, and a roomy status box). The daemon also
//! runs headless; this is only a view.
use construct_core::store::{RunRecord, Store};
use construct_engine::events::{EngineEvent, EventKind};
use construct_store::SqliteStore;
use crossterm::event::{self, Event, KeyCode};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph};
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

/// The logo box content: the 🌐 brand mark, the name, the publisher, and the
/// tagline. (A terminal can't render docs/globe.png, so the globe emoji stands in.)
// design-todo: see docs/design-todo.md.
fn logo_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::raw("🌐  "),
            Span::styled(
                "THE CONSTRUCT",
                Style::default().fg(NEON).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            "    websites on computers",
            Style::default().fg(CYAN),
        )),
        Line::from(Span::styled(
            "    the folder is the prompt",
            Style::default().fg(DIM),
        )),
    ]
}

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
    /// Inbox folder + idle minutes, when the inbox feature is on.
    pub inbox: Option<(String, u64)>,
    /// Where tracing logs are being written (so the user can `tail` them).
    pub log_path: Option<String>,
    /// The config file, for the in-TUI view (`c`) and edit (`e`) commands.
    pub config_path: std::path::PathBuf,
}

struct State {
    activity: VecDeque<(EventKind, String, String)>, // kind, time, message
    recent: Vec<RunRecord>,                          // recently processed notes
    processed: u64,                                  // lifetime events seen
    det: u64,                                        // notes handled with NO model
    model: u64,                                      // notes that escalated to a model
    pending_review: usize,
    started: Instant,
    paused: bool,
    frame: u64,                         // render tick, drives the spinner flourish
    config_view: Option<(String, u16)>, // raw config text + scroll offset, when open
    flash: Option<(String, Instant)>,   // transient status message
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
        det: 0,
        model: 0,
        pending_review: 0,
        started: Instant::now(),
        paused: false,
        frame: 0,
        config_view: None,
        flash: None,
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
                    // Tally completions for the deterministic-first meter. Each run
                    // emits one " done" event; count it by how it was handled.
                    if ev.message.contains(" done") {
                        match ev.kind {
                            EventKind::Deterministic => state.det = state.det.saturating_add(1),
                            EventKind::Run
                            | EventKind::Inbox
                            | EventKind::Brief
                            | EventKind::Daily => state.model = state.model.saturating_add(1),
                            _ => {}
                        }
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

        state.frame = state.frame.wrapping_add(1);
        if let Err(e) = terminal.draw(|f| draw(f, &ctx, &state)) {
            break Err(e.into());
        }

        let polled = match event::poll(Duration::from_millis(200)) {
            Ok(p) => p,
            Err(e) => break Err(e.into()),
        };
        if polled {
            match event::read() {
                Ok(Event::Key(key)) => {
                    // When the config viewer is open, keys drive it instead.
                    if let Some((_, scroll)) = &mut state.config_view {
                        match key.code {
                            KeyCode::Char('c') | KeyCode::Char('q') | KeyCode::Esc => {
                                state.config_view = None
                            }
                            KeyCode::Char('e') => {
                                edit_config(&mut terminal, &ctx.config_path, &mut state.flash);
                                state.config_view = None;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                *scroll = scroll.saturating_add(1)
                            }
                            KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                            _ => {}
                        }
                        continue;
                    }
                    match key.code {
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
                        KeyCode::Char('c') => {
                            let text = std::fs::read_to_string(&ctx.config_path)
                                .unwrap_or_else(|e| format!("could not read config: {e}"));
                            state.config_view = Some((text, 0));
                        }
                        KeyCode::Char('e') => {
                            edit_config(&mut terminal, &ctx.config_path, &mut state.flash)
                        }
                        _ => {}
                    }
                }
                Ok(_) => {}
                Err(e) => break Err(e.into()),
            }
        }
    };
    ratatui::restore();
    res
}

/// Suspend the TUI, open the config in `$VISUAL`/`$EDITOR` (fallback `vi`), then
/// re-enter. The daemon already loaded its config, so changes apply on the next
/// `construct watch` — we flash a reminder.
fn edit_config(
    terminal: &mut ratatui::DefaultTerminal,
    config_path: &std::path::Path,
    flash: &mut Option<(String, Instant)>,
) {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    ratatui::restore(); // leave the alternate screen so the editor owns the terminal
    let status = std::process::Command::new(&editor)
        .arg(config_path)
        .status();
    *terminal = ratatui::init(); // re-enter the alternate screen
    let _ = terminal.clear();
    *flash = Some((
        match status {
            Ok(s) if s.success() => "config saved · restart `construct watch` to apply".to_string(),
            _ => format!("couldn't launch editor `{editor}` (set $EDITOR)"),
        },
        Instant::now(),
    ));
}

/// Braille spinner — the cheap "uplink alive" flourish; advances each render tick.
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

fn draw(f: &mut Frame, ctx: &DashboardCtx, state: &State) {
    // Title bar, main split, then a thin footer (flourish + flash).
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_title(f, ctx, state, rows[0]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(rows[1]);

    draw_left(f, cols[0], state); // two stacked panels: Activity + Recent Notes
    draw_right(f, ctx, state, cols[1]); // logo, meter, keys, status
    draw_footer(f, state, rows[2]);

    // Config viewer overlays everything when open.
    if let Some((text, scroll)) = &state.config_view {
        draw_config_overlay(f, ctx, text, *scroll);
    }
}

/// Footer: an animated "uplink" flourish on the left, and a transient flash
/// message (e.g. after editing the config) for ~4s on the right.
fn draw_footer(f: &mut Frame, state: &State, area: Rect) {
    let spin = SPINNER[(state.frame as usize) % SPINNER.len()];
    let mut spans = vec![
        Span::styled(format!(" {spin} "), Style::default().fg(NEON)),
        Span::styled("uplink ", Style::default().fg(DIM)),
        Span::styled(
            "c config · e edit · o open · p pause · q quit",
            Style::default().fg(DIM),
        ),
    ];
    if let Some((msg, at)) = &state.flash {
        if at.elapsed() < Duration::from_secs(4) {
            spans.push(Span::styled("   ▸ ", Style::default().fg(AMBER)));
            spans.push(Span::styled(
                msg.clone(),
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
            ));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Centered modal showing the raw config file with a scroll offset.
fn draw_config_overlay(f: &mut Frame, ctx: &DashboardCtx, text: &str, scroll: u16) {
    let area = centered(f.area(), 80, 80);
    f.render_widget(Clear, area);
    let title = format!(" CONFIG · {} ", ctx.config_path.display());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(NEON))
        .title(Span::styled(
            title,
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " ↑/↓ scroll · e edit · c/esc close ",
            Style::default().fg(DIM),
        ));
    let body = Paragraph::new(text)
        .style(Style::default().fg(FG))
        .block(block)
        .scroll((scroll, 0));
    f.render_widget(body, area);
}

/// A rectangle centered in `area`, sized to the given width/height percentages.
fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(v[1])[1]
}

fn draw_title(f: &mut Frame, _ctx: &DashboardCtx, state: &State, area: Rect) {
    let (dot, dot_color, word) = if state.paused {
        ('⏸', AMBER, "paused")
    } else {
        ('●', NEON, "watching")
    };
    let up = state.started.elapsed().as_secs();
    let line = Line::from(vec![
        Span::raw(" 🌐 "),
        Span::styled(
            "THE CONSTRUCT ",
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

/// Right column: Logo, the deterministic-first meter, Commands, then a roomy
/// Status box (stacked).
fn draw_right(f: &mut Frame, ctx: &DashboardCtx, state: &State, area: Rect) {
    let boxes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // logo
            Constraint::Length(6), // deterministic-first meter (≈ logo size)
            Constraint::Length(7), // commands
            Constraint::Min(6),    // status — the largest box, fills the rest
        ])
        .split(area);

    // --- Logo ---
    let logo = Paragraph::new(logo_lines()).block(panel("🌐", false));
    f.render_widget(logo, boxes[0]);

    // --- Deterministic-first meter (the thesis, quantified) ---
    f.render_widget(meter_widget(state), boxes[1]);

    // --- Commands ---
    let key = |k: &str, label: &str| {
        Line::from(vec![
            Span::styled(format!(" {k} "), Style::default().fg(Color::Black).bg(CYAN)),
            Span::styled(format!("  {label}"), Style::default().fg(FG)),
        ])
    };
    let commands = Paragraph::new(vec![
        key("c", "view config"),
        key("e", "edit config"),
        key("o", "open today's note"),
        key("p", "pause / resume"),
        key("q", "quit"),
    ])
    .block(panel("KEYS", false));
    f.render_widget(commands, boxes[2]);

    // --- Status ---
    f.render_widget(status_widget(ctx, state), boxes[3]);
}

/// The deterministic-first meter: how many notes were handled with NO model vs
/// escalated, and the local-handling percentage with a little bar. This is the
/// product's whole claim, made live — and the number a self-hoster wants to brag
/// about ("96% handled locally, zero tokens").
fn meter_widget(state: &State) -> Paragraph<'static> {
    let total = state.det + state.model;
    // 100% local when nothing has escalated yet (total == 0 → checked_div is None).
    let pct = state
        .det
        .saturating_mul(100)
        .checked_div(total)
        .unwrap_or(100);
    // 10-cell bar.
    let filled = (pct / 10) as usize;
    let bar: String = "█".repeat(filled) + &"░".repeat(10 - filled);
    let bar_color = if pct >= 80 {
        NEON
    } else if pct >= 50 {
        GREEN
    } else {
        AMBER
    };
    let row = |label: &'static str, value: String, color: Color| {
        Line::from(vec![
            Span::styled(format!(" {label:<10}"), Style::default().fg(DIM)),
            Span::styled(value, Style::default().fg(color)),
        ])
    };
    Paragraph::new(vec![
        row("no-model", state.det.to_string(), NEON),
        row("escalated", state.model.to_string(), FG),
        Line::from(vec![
            Span::styled(format!(" {bar} "), Style::default().fg(bar_color)),
            Span::styled(
                format!("{pct}% local"),
                Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
            ),
        ]),
    ])
    .block(panel("DETERMINISTIC", false))
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
    if let Some((folder, idle)) = &ctx.inbox {
        lines.push(row("inbox", format!("{folder}/ · {idle}m idle"), FG));
    }
    if let Some(t) = &ctx.daily_time {
        lines.push(row("daily", t.clone(), FG));
    }
    if let Some(b) = &ctx.briefs_folder {
        lines.push(row("briefs", format!("{b}/"), FG));
    }
    if let Some(log) = &ctx.log_path {
        lines.push(row("logs", tilde(log), DIM));
    }
    Paragraph::new(lines).block(panel("STATUS", true))
}

/// Shorten a home-relative path to `~/…` for display.
fn tilde(p: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if p.starts_with(&home) => format!("~{}", &p[home.len()..]),
        _ => p.to_string(),
    }
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
