CREATE TABLE controls (
    stream_id TEXT PRIMARY KEY NOT NULL REFERENCES streams(id),
    data TEXT NOT NULL CHECK(json_valid(data))
);
PRAGMA user_version=4;
