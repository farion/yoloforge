CREATE TABLE IF NOT EXISTS validations (
    source_id TEXT NOT NULL REFERENCES commands(id),
    ticket_id TEXT NOT NULL REFERENCES tickets(id),
    reviewer_id TEXT NOT NULL PRIMARY KEY REFERENCES commands(id),
    rework_id TEXT UNIQUE REFERENCES commands(id),
    followup TEXT,
    retired INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX IF NOT EXISTS validations_current_source ON validations(source_id) WHERE retired=0;
CREATE INDEX IF NOT EXISTS validations_ticket ON validations(ticket_id);
CREATE TABLE IF NOT EXISTS validation_controllers (
    project_id TEXT NOT NULL REFERENCES projects(id),
    runner_id TEXT NOT NULL,
    stream_id TEXT NOT NULL UNIQUE REFERENCES streams(id),
    PRIMARY KEY (project_id, runner_id)
);
PRAGMA user_version=43;
