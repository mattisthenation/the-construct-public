//! Unified trigger events. Any source (vault watcher, idle poller, scheduler)
//! produces a `TriggerEvent`; the watch loop routes it to an orchestrator.
pub mod idle;
pub mod schedule;

use construct_obsidian::watcher::VaultEvent;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum TriggerEvent {
    /// A note gained a known trigger tag (today's `NoteTagged`).
    Tagged { path: PathBuf, tag: String },
    /// A note's `construct_status` changed to accepted/rejected (today's `StatusChanged`).
    StatusChanged { path: PathBuf, status: String },
    /// An Inbox note has been idle long enough to process (Plan 2).
    IdleNote { path: PathBuf },
    /// A named scheduled job is due to run now (Plan 3), e.g. "daily_summary".
    Scheduled { job: String },
    /// A Daily Brief file changed (Slice 4).
    Brief {
        path: PathBuf,
        date: chrono::NaiveDate,
    },
}

impl From<VaultEvent> for TriggerEvent {
    fn from(e: VaultEvent) -> Self {
        match e {
            VaultEvent::NoteTagged { path, tag } => TriggerEvent::Tagged { path, tag },
            VaultEvent::StatusChanged { path, status } => {
                TriggerEvent::StatusChanged { path, status }
            }
            VaultEvent::BriefChanged { path, date } => TriggerEvent::Brief { path, date },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_event_maps_to_trigger_event() {
        let ev = VaultEvent::NoteTagged {
            path: PathBuf::from("/v/a.md"),
            tag: "theconstruct/research".into(),
        };
        assert_eq!(
            TriggerEvent::from(ev),
            TriggerEvent::Tagged {
                path: PathBuf::from("/v/a.md"),
                tag: "theconstruct/research".into()
            }
        );
        let ev = VaultEvent::StatusChanged {
            path: PathBuf::from("/v/a.md"),
            status: "accepted".into(),
        };
        assert_eq!(
            TriggerEvent::from(ev),
            TriggerEvent::StatusChanged {
                path: PathBuf::from("/v/a.md"),
                status: "accepted".into()
            }
        );
    }
}
