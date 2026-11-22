CREATE TABLE IF NOT EXISTS archive_blobs (
    command_id TEXT NOT NULL REFERENCES commands(id),
    sha256 TEXT NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data)),
    bytes INTEGER NOT NULL,
    received INTEGER NOT NULL DEFAULT 0,
    complete INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(command_id, sha256)
);
CREATE TABLE IF NOT EXISTS archive_blob_chunks (
    command_id TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    offset INTEGER NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY(command_id, sha256, offset),
    FOREIGN KEY(command_id,sha256) REFERENCES archive_blobs(command_id,sha256)
);
PRAGMA user_version=31;
