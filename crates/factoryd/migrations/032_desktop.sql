CREATE TABLE IF NOT EXISTS notifications (
    id INTEGER PRIMARY KEY,
    dedup TEXT NOT NULL UNIQUE,
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE IF NOT EXISTS notification_reads (
    owner TEXT NOT NULL,
    notification_id INTEGER NOT NULL REFERENCES notifications(id),
    PRIMARY KEY(owner,notification_id)
);
PRAGMA user_version=32;
