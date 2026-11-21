CREATE TABLE supervisor_policies (
    project_id TEXT PRIMARY KEY NOT NULL REFERENCES projects(id),
    stream_id TEXT UNIQUE NOT NULL REFERENCES streams(id),
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE supervisor_evaluations (
    command_id TEXT PRIMARY KEY NOT NULL REFERENCES commands(id),
    project_id TEXT NOT NULL REFERENCES projects(id),
    target_command_id TEXT NOT NULL REFERENCES commands(id),
    resolved INTEGER NOT NULL DEFAULT 0,
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX supervisor_pending ON supervisor_evaluations(project_id,resolved);
PRAGMA user_version=11;
