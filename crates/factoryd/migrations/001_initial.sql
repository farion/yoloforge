CREATE TABLE projects (
    id TEXT PRIMARY KEY NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE tickets (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id),
    revision INTEGER NOT NULL CHECK(revision > 0),
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX tickets_project ON tickets(project_id);
CREATE TABLE requests (
    id TEXT PRIMARY KEY NOT NULL,
    fingerprint TEXT NOT NULL,
    response TEXT NOT NULL CHECK(json_valid(response))
);
CREATE TABLE events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    created_unix_ms INTEGER NOT NULL,
    payload TEXT NOT NULL CHECK(json_valid(payload))
);
PRAGMA user_version=1;
