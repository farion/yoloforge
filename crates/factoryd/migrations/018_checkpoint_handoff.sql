CREATE TABLE checkpoint_bundles (
    command_id TEXT PRIMARY KEY NOT NULL REFERENCES checkpoints(command_id),
    sha256 TEXT NOT NULL,
    size INTEGER NOT NULL,
    content BLOB NOT NULL
);
PRAGMA user_version=18;
