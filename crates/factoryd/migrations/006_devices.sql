CREATE TABLE devices (
    id TEXT PRIMARY KEY NOT NULL,
    token_hash TEXT UNIQUE NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data)),
    revoked INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE enrollments (
    code_hash TEXT PRIMARY KEY NOT NULL,
    expires_unix_ms INTEGER NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data))
);
PRAGMA user_version=6;
