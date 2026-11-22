CREATE TABLE IF NOT EXISTS token_budgets(project_id TEXT PRIMARY KEY REFERENCES projects(id), data TEXT NOT NULL);
PRAGMA user_version=30;
