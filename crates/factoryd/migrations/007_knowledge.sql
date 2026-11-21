CREATE TABLE knowledge (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id),
    ticket_id TEXT REFERENCES tickets(id),
    retired INTEGER NOT NULL DEFAULT 0,
    priority INTEGER NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX knowledge_project ON knowledge(project_id,retired,priority);
PRAGMA user_version=7;
