-- Content-hash guard for Daily Briefs: re-saving an unchanged brief must not
-- re-trigger the (token-spending) recap agent.
CREATE TABLE IF NOT EXISTS brief_state (
    path       TEXT PRIMARY KEY,
    hash       TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
