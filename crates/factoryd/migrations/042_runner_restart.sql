CREATE TABLE IF NOT EXISTS runner_restart_holds (
    runner_id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    expires_ms INTEGER NOT NULL
);
PRAGMA user_version=42;
