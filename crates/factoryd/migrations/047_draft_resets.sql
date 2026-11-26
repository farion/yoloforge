CREATE TABLE IF NOT EXISTS draft_resets (
    request_id TEXT PRIMARY KEY,
    ticket_id TEXT NOT NULL REFERENCES tickets(id),
    stream_id TEXT NOT NULL UNIQUE REFERENCES streams(id),
    root_id TEXT NOT NULL UNIQUE REFERENCES commands(id),
    runner_id TEXT NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0 CHECK(completed IN (0,1)),
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX IF NOT EXISTS draft_resets_runner ON draft_resets(runner_id,completed);
PRAGMA user_version=47;
