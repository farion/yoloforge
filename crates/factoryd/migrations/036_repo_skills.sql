CREATE TABLE IF NOT EXISTS project_skills (
  project_id TEXT PRIMARY KEY REFERENCES projects(id),
  data TEXT NOT NULL CHECK (json_valid(data))
);
CREATE TABLE IF NOT EXISTS task_skills (
  ticket_id TEXT PRIMARY KEY REFERENCES tickets(id),
  data TEXT NOT NULL CHECK (json_valid(data))
);
PRAGMA user_version=36;
