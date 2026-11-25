CREATE TABLE IF NOT EXISTS local_mcp_catalog (
  id INTEGER PRIMARY KEY CHECK(id=1),
  data TEXT NOT NULL CHECK (json_valid(data))
);
CREATE TABLE IF NOT EXISTS project_local_mcps (
  project_id TEXT PRIMARY KEY REFERENCES projects(id),
  data TEXT NOT NULL CHECK (json_valid(data))
);
CREATE TABLE IF NOT EXISTS task_local_mcps (
  ticket_id TEXT PRIMARY KEY REFERENCES tickets(id),
  data TEXT NOT NULL CHECK (json_valid(data))
);
PRAGMA user_version=37;
