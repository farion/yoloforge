CREATE TABLE archive_status (
    command_id TEXT PRIMARY KEY NOT NULL REFERENCES commands(id),
    fingerprint TEXT NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE archive_messages (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    command_id TEXT NOT NULL REFERENCES commands(id),
    message_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    sample_revision INTEGER NOT NULL,
    captured_unix_ms INTEGER NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX archive_command ON archive_messages(command_id,sequence);
CREATE INDEX archive_message_version ON archive_messages(command_id,message_id,sequence);
PRAGMA user_version=8;
