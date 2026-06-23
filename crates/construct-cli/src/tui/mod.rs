pub mod chat;
pub mod dashboard;
pub mod watch_loop;

use crate::theme::Theme;
use chat::ChatState;
use construct_config::Config;
use construct_model_ollama::OllamaProvider;
use crossterm::event::{self, Event, KeyCode};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};
use std::sync::Arc;
use std::time::Duration;

/// Launch the TUI: header, runs panel (left), chat (right).
pub async fn run_tui(cfg: Option<Config>) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    // Prefer a notes/Librarian-style agent for chat (smaller, no web tools) over
    // whatever happens to be first (e.g. a heavyweight research model). Fall back
    // to the first agent, then to a sane local default.
    let chat_agent = cfg.as_ref().and_then(|c| {
        c.agents
            .iter()
            .find(|a| a.domain == "notes" || a.tools.is_empty())
            .or_else(|| c.agents.first())
    });
    let (model, base_url, system) = match chat_agent {
        Some(a) => (
            a.model.clone(),
            a.base_url.clone(),
            format!("You are {}.", a.name),
        ),
        None => (
            "llama3.1".into(),
            "http://localhost:11434".into(),
            "You are a helpful assistant.".into(),
        ),
    };
    let provider: Arc<dyn construct_core::model::ModelProvider> =
        Arc::new(OllamaProvider::new(base_url));
    let mut chat = ChatState::new(model, &system);

    loop {
        terminal.draw(|f| draw(f, &chat))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Esc => break,
                    KeyCode::Char(c) => chat.push_char(c),
                    KeyCode::Backspace => chat.backspace(),
                    KeyCode::Enter if chat.take_input().is_some() => {
                        // Show a "thinking…" hint, then do a blocking send (async
                        // streaming is a later slice). Errors surface in the transcript.
                        chat.thinking = true;
                        terminal.draw(|f| draw(f, &chat))?;
                        chat.send(provider.clone()).await;
                        chat.thinking = false;
                    }
                    _ => {}
                }
            }
        }
    }
    ratatui::restore();
    Ok(())
}

fn draw(f: &mut Frame, chat: &ChatState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    let header = Paragraph::new("  The Construct ").style(Theme::header());
    f.render_widget(header, chunks[0]);

    let transcript: Vec<Line> = chat
        .visible()
        .iter()
        .map(|m| Line::from(format!("{:?}: {}", m.role, m.content)).style(Theme::body()))
        .collect();
    let body = Paragraph::new(transcript).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Chat")
            .border_style(Theme::accent()),
    );
    f.render_widget(body, chunks[1]);

    let input_title = if chat.thinking {
        "thinking…"
    } else {
        "Type (Esc to quit)"
    };
    let input = Paragraph::new(chat.input.as_str())
        .style(Theme::body())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(input_title)
                .border_style(Theme::accent()),
        );
    f.render_widget(input, chunks[2]);
}
