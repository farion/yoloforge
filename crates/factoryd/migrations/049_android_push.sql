CREATE TABLE IF NOT EXISTS android_push_devices (
    id TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    sender TEXT NOT NULL,
    token TEXT NOT NULL UNIQUE,
    retry_after INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS android_push_delivered (
    device TEXT NOT NULL REFERENCES android_push_devices(id) ON DELETE CASCADE,
    entry TEXT NOT NULL,
    PRIMARY KEY (device, entry)
);
PRAGMA user_version=49;
