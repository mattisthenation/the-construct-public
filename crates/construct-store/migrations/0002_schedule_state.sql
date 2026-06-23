CREATE TABLE IF NOT EXISTS schedule_state (
    job       TEXT PRIMARY KEY,
    last_run  TEXT NOT NULL
);
