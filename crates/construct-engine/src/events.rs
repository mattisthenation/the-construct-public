//! Engine → UI event stream. The engine publishes; consumers (the dashboard)
//! subscribe via tokio broadcast. Lossy by design: a slow/absent UI must never
//! block or break pipelines (broadcast drops oldest on overflow).
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Inbox,
    Brief,
    Daily,
    Run,
    Error,
    Info,
}

impl EventKind {
    pub fn label(&self) -> &'static str {
        match self {
            EventKind::Inbox => "inbox",
            EventKind::Brief => "brief",
            EventKind::Daily => "daily",
            EventKind::Run => "run",
            EventKind::Error => "error",
            EventKind::Info => "info",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EngineEvent {
    pub kind: EventKind,
    pub message: String,
    /// Local wall-clock HH:MM:SS, formatted at emit time.
    pub time: String,
}

pub type EventSender = broadcast::Sender<EngineEvent>;

/// Create the channel. 256 is plenty: the dashboard drains continuously and
/// only a wall of simultaneous runs could overflow (oldest dropped, by design).
pub fn channel() -> (EventSender, broadcast::Receiver<EngineEvent>) {
    broadcast::channel(256)
}

/// Fire-and-forget emit; never errors (no subscribers is fine).
pub fn emit(tx: &EventSender, kind: EventKind, message: impl Into<String>) {
    let _ = tx.send(EngineEvent {
        kind,
        message: message.into(),
        time: chrono::Local::now().format("%H:%M:%S").to_string(),
    });
}
