CREATE TABLE IF NOT EXISTS ticket_counters (
    project_id TEXT PRIMARY KEY REFERENCES projects(id),
    last_number INTEGER NOT NULL DEFAULT 0 CHECK(last_number >= 0)
);
CREATE TABLE IF NOT EXISTS project_key_aliases (
    key TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id)
);
CREATE UNIQUE INDEX IF NOT EXISTS projects_ticket_key
ON projects(json_extract(data, '$.key'))
WHERE COALESCE(json_extract(data, '$.key'), '') <> '';
CREATE UNIQUE INDEX IF NOT EXISTS tickets_human_key
ON tickets(json_extract(data, '$.key'))
WHERE COALESCE(json_extract(data, '$.key'), '') <> '';
PRAGMA user_version=34;
