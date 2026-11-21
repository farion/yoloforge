CREATE TABLE scheduler_policies (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES projects(id),
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE scheduler_runs (
    ticket_id TEXT PRIMARY KEY NOT NULL REFERENCES tickets(id),
    stream_id TEXT NOT NULL REFERENCES streams(id),
    prepare_id TEXT NOT NULL REFERENCES commands(id),
    agent_id TEXT REFERENCES commands(id),
    prompt TEXT NOT NULL
);
PRAGMA user_version=20;
