CREATE TABLE IF NOT EXISTS factory_model_settings (
    id INTEGER PRIMARY KEY CHECK(id=1),
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE IF NOT EXISTS project_model_settings (
    project_id TEXT PRIMARY KEY REFERENCES projects(id),
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE IF NOT EXISTS task_models (
    ticket_id TEXT PRIMARY KEY REFERENCES tickets(id),
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE IF NOT EXISTS model_access_bindings (
    access_id TEXT PRIMARY KEY,
    credential_id TEXT NOT NULL UNIQUE
);
PRAGMA user_version=35;
