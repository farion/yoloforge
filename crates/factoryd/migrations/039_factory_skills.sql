CREATE TABLE IF NOT EXISTS factory_skill_versions (
  id TEXT NOT NULL,
  version INTEGER NOT NULL,
  summary TEXT NOT NULL CHECK(json_valid(summary)),
  files TEXT NOT NULL CHECK(json_valid(files)),
  PRIMARY KEY(id,version)
);
PRAGMA user_version=39;
