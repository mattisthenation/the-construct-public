CREATE TABLE IF NOT EXISTS runs (
    id          TEXT PRIMARY KEY,
    rule        TEXT NOT NULL,
    agent       TEXT NOT NULL,
    note_path   TEXT NOT NULL,
    status      TEXT NOT NULL,
    error       TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_runs_note ON runs(note_path);

CREATE TABLE IF NOT EXISTS run_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id      TEXT NOT NULL REFERENCES runs(id),
    stage       TEXT NOT NULL,
    event       TEXT NOT NULL,
    payload     TEXT NOT NULL,
    ts          TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_events_run ON run_events(run_id);
