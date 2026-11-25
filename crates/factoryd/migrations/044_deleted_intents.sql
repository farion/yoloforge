CREATE TABLE IF NOT EXISTS deleted_intents (
    ticket_id TEXT PRIMARY KEY REFERENCES tickets(id)
);
PRAGMA user_version=44;
