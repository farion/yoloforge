CREATE TABLE IF NOT EXISTS remote_mcps(
  id TEXT PRIMARY KEY,
  data TEXT NOT NULL CHECK(json_valid(data)),
  sealed BLOB
);
PRAGMA user_version=38;
