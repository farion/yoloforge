-- Older automatically extracted observations lacked branch validity. Keep their
-- provenance, but retire them instead of silently treating them as project-wide.
CREATE TEMP TABLE unscoped_supervisor_knowledge AS
SELECT k.id FROM knowledge k WHERE k.retired=0 AND json_extract(k.data,'$.branch') IS NULL
AND EXISTS(SELECT 1 FROM supervisor_evaluations e WHERE json_extract(e.data,'$.knowledge_id')=k.id);
UPDATE knowledge SET retired=1,
data=json_set(data,'$.retired',json('true'),'$.revision',json_extract(data,'$.revision')+1)
WHERE id IN (SELECT id FROM unscoped_supervisor_knowledge);
INSERT INTO events(kind,aggregate_id,created_unix_ms,payload)
SELECT 'knowledge.retired_for_scope_migration',id,CAST(strftime('%s','now') AS INTEGER)*1000,data
FROM knowledge WHERE id IN (SELECT id FROM unscoped_supervisor_knowledge);
DROP TABLE unscoped_supervisor_knowledge;
CREATE INDEX knowledge_branch_scope ON knowledge(project_id,retired,json_extract(data,'$.branch'));
PRAGMA user_version=12;
