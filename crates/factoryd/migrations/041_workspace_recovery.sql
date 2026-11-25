CREATE TABLE IF NOT EXISTS workspace_recoveries (
    command_id TEXT PRIMARY KEY REFERENCES commands(id),
    ticket_id TEXT NOT NULL REFERENCES tickets(id),
    data TEXT NOT NULL CHECK(json_valid(data))
);
PRAGMA user_version=41;
