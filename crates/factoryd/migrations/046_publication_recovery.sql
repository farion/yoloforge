CREATE TABLE IF NOT EXISTS publication_recovery_holds (
    project_id TEXT PRIMARY KEY NOT NULL,
    command_id TEXT NOT NULL UNIQUE,
    runner_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    expires_ms INTEGER NOT NULL
);
PRAGMA user_version=46;
