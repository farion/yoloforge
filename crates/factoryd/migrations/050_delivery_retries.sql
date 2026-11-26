CREATE TABLE IF NOT EXISTS delivery_retries (
    ticket_id TEXT NOT NULL REFERENCES tickets(id),
    step TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    next_ms INTEGER NOT NULL,
    PRIMARY KEY(ticket_id, step)
);
PRAGMA user_version=50;
