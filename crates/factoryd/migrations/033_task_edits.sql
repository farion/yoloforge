CREATE TABLE IF NOT EXISTS task_edits (
    ticket_id TEXT PRIMARY KEY REFERENCES tickets(id),
    token TEXT NOT NULL UNIQUE
);
CREATE INDEX IF NOT EXISTS events_aggregate_kind ON events(aggregate_id,kind);
PRAGMA user_version=33;
