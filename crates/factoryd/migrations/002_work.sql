CREATE TABLE streams (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id),
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE stream_tickets (
    ticket_id TEXT PRIMARY KEY NOT NULL REFERENCES tickets(id),
    stream_id TEXT NOT NULL REFERENCES streams(id)
);
CREATE TABLE commands (
    id TEXT PRIMARY KEY NOT NULL,
    stream_id TEXT NOT NULL REFERENCES streams(id),
    runner_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('pending','accepted','succeeded','failed')),
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE UNIQUE INDEX one_active_command_per_stream ON commands(stream_id)
    WHERE state IN ('pending','accepted');
CREATE INDEX runner_commands ON commands(runner_id,state);
PRAGMA user_version=2;
