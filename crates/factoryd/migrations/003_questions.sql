CREATE TABLE questions (
    id TEXT PRIMARY KEY NOT NULL,
    command_id TEXT NOT NULL REFERENCES commands(id),
    kind TEXT NOT NULL,
    external_id TEXT NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data)),
    UNIQUE(command_id,kind,external_id)
);
PRAGMA user_version=3;
