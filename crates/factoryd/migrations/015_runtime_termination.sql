CREATE TABLE runtime_terminations (
    command_id TEXT PRIMARY KEY NOT NULL REFERENCES commands(id),
    stream_id TEXT NOT NULL REFERENCES streams(id),
    state TEXT NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX runtime_terminations_stream ON runtime_terminations(stream_id,state);
PRAGMA user_version=15;
