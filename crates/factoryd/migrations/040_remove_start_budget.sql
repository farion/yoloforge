UPDATE scheduler_policies
SET data = json_remove(data, '$.remaining_starts')
WHERE json_type(data, '$.remaining_starts') IS NOT NULL;

PRAGMA user_version=40;
