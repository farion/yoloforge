CREATE TABLE session_closures (
    root_command_id TEXT PRIMARY KEY NOT NULL REFERENCES commands(id),
    data TEXT NOT NULL CHECK(json_valid(data))
);
PRAGMA user_version=10;
