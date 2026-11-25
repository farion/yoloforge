CREATE TABLE IF NOT EXISTS directs (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    stream_id TEXT UNIQUE REFERENCES streams(id),
    data TEXT NOT NULL CHECK(json_valid(data)),
    pending_turn TEXT CHECK(pending_turn IS NULL OR json_valid(pending_turn))
);
CREATE INDEX IF NOT EXISTS directs_project ON directs(project_id);
CREATE TABLE IF NOT EXISTS direct_intents (
    question_id TEXT PRIMARY KEY REFERENCES questions(id),
    ticket_id TEXT NOT NULL UNIQUE REFERENCES tickets(id)
);
PRAGMA user_version=43;
