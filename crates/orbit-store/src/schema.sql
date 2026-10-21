CREATE TABLE settings (key TEXT PRIMARY KEY NOT NULL, value BLOB NOT NULL);
CREATE TABLE jobs (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'retry_wait', 'succeeded', 'blocked', 'cancelled', 'unknown')),
    created_at INTEGER NOT NULL,
    payload BLOB NOT NULL
);
CREATE INDEX jobs_state_time ON jobs(state, created_at);
CREATE TABLE chunks (
    id TEXT PRIMARY KEY NOT NULL,
    recording_id TEXT NOT NULL,
    track TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    bytes INTEGER NOT NULL CHECK (bytes > 0),
    payload BLOB NOT NULL,
    UNIQUE (recording_id, track, sequence)
);
