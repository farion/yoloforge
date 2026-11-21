CREATE TABLE compactions (
    request_id TEXT PRIMARY KEY NOT NULL,
    command_id TEXT NOT NULL REFERENCES commands(id),
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX compaction_command ON compactions(command_id);
PRAGMA user_version=9;
