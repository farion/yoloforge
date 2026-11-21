use anyhow::{ensure, Result};
use factory_core::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};

pub struct Journal {
    pub(crate) db: Connection,
    _lock: File,
}
impl Drop for Journal {
    fn drop(&mut self) {
        // Explicit unlock also releases the lock if a concurrent fork briefly
        // inherited the descriptor before close-on-exec. Closing alone can keep
        // the lock alive until that child reaches exec.
        let _ = fs2::FileExt::unlock(&self._lock);
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub command: RunnerCommand,
    pub ack_request: RequestId,
    pub result_request: RequestId,
    pub result: Option<CommandOutput>,
    pub reported: bool,
    #[serde(default)]
    pub agent: Option<crate::agent::Execution>,
    #[serde(default)]
    pub excerpt: Option<SessionExcerpt>,
    #[serde(default)]
    pub archive: crate::archive::Cursor,
    #[serde(default)]
    pub compactions: std::collections::BTreeMap<RequestId, crate::compaction::Receipt>,
    #[serde(default)]
    pub question_snapshot: Option<Vec<ExternalQuestion>>,
    #[serde(default)]
    pub replies: std::collections::BTreeMap<RequestId, crate::questions::Receipt>,
}
impl Journal {
    pub fn open(path: &Path, server: &str, runner: RunnerId) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)?;
        let mut db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(3))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        let tx = db.transaction()?;
        let version: u32 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(version <= 2, "runner journal schema is too new");
        if version == 0 {
            tx.execute_batch("CREATE TABLE identity(id INTEGER PRIMARY KEY CHECK(id=1),server TEXT NOT NULL,runner TEXT NOT NULL);
                CREATE TABLE commands(id TEXT PRIMARY KEY NOT NULL,data TEXT NOT NULL CHECK(json_valid(data)));
                PRAGMA user_version=1;")?;
            tx.execute(
                "INSERT INTO identity VALUES(1,?1,?2)",
                params![server.trim_end_matches('/'), runner.to_string()],
            )?;
        }
        if version < 2 {
            tx.execute_batch("CREATE TABLE workspaces(stream_id TEXT PRIMARY KEY NOT NULL,data TEXT NOT NULL CHECK(json_valid(data))); PRAGMA user_version=2;")?;
        }
        let (saved_server, saved_runner): (String, String) =
            tx.query_row("SELECT server,runner FROM identity WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        ensure!(
            saved_server == server.trim_end_matches('/') && saved_runner == runner.to_string(),
            "journal belongs to a different runner or server"
        );
        tx.commit()?;
        Ok(Self { db, _lock: lock })
    }
    pub fn receive(&self, command: RunnerCommand) -> Result<Record> {
        let previous: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM commands WHERE id=?1",
                [command.id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            let record: Record = serde_json::from_str(&previous)?;
            let old = &record.command;
            ensure!(
                old.runner_id == command.runner_id
                    && old.stream_id == command.stream_id
                    && old.ownership_epoch == command.ownership_epoch
                    && old.kind == command.kind
                    && old.workspace_spec == command.workspace_spec
                    && old.agent_spec == command.agent_spec
                    && old.created_unix_ms == command.created_unix_ms,
                "command ID reused with different immutable payload"
            );
            return Ok(record);
        }
        let record = Record {
            command,
            ack_request: RequestId::new(),
            result_request: RequestId::new(),
            result: None,
            reported: false,
            agent: None,
            question_snapshot: None,
            replies: Default::default(),
            excerpt: None,
            archive: Default::default(),
            compactions: Default::default(),
        };
        self.save(&record)?;
        Ok(record)
    }
    pub fn record(&self, id: CommandId) -> Result<Option<Record>> {
        let data: Option<String> = self
            .db
            .query_row(
                "SELECT data FROM commands WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }
    pub fn save(&self, record: &Record) -> Result<()> {
        self.db.execute("INSERT INTO commands(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
            params![record.command.id.to_string(),serde_json::to_string(record)?])?;
        Ok(())
    }
    pub fn unreported(&self) -> Result<Vec<Record>> {
        let mut stmt = self.db.prepare("SELECT data FROM commands WHERE json_extract(data,'$.reported')=0 AND json_extract(data,'$.result') IS NOT NULL ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(serde_json::from_str(&row?)?);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrates_v1_journal_without_losing_legacy_results() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runner.db");
        let runner = RunnerId::new();
        let id = CommandId::new();
        let legacy = serde_json::json!({
            "command": {"id":id,"stream_id":StreamId::new(),"runner_id":runner,
                "ownership_epoch":1,"kind":"check_open_code","state":"accepted",
                "result":null,"created_unix_ms":1},
            "ack_request":RequestId::new(),"result_request":RequestId::new(),
            "result":{"status":"not_configured"},"reported":false
        });
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE identity(id INTEGER PRIMARY KEY,server TEXT,runner TEXT); CREATE TABLE commands(id TEXT PRIMARY KEY,data TEXT); PRAGMA user_version=1;").unwrap();
        db.execute(
            "INSERT INTO identity VALUES(1,?1,?2)",
            params!["http://localhost:7432", runner.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO commands VALUES(?1,?2)",
            params![id.to_string(), legacy.to_string()],
        )
        .unwrap();
        drop(db);
        let journal = Journal::open(&path, "http://localhost:7432", runner).unwrap();
        let outbox = journal.unreported().unwrap();
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].result, Some(OpenCodeStatus::NotConfigured.into()));
        assert!(outbox[0].command.workspace_spec.is_none());
        let count: u64 = journal
            .db
            .query_row("SELECT COUNT(*) FROM workspaces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    #[test]
    fn restart_keeps_receipts_and_results_and_rejects_payload_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runner.db");
        let runner = RunnerId::new();
        let command = RunnerCommand {
            id: CommandId::new(),
            stream_id: StreamId::new(),
            runner_id: runner,
            ownership_epoch: 1,
            kind: CommandKind::CheckOpenCode,
            state: CommandState::Pending,
            result: None,
            created_unix_ms: 1,
            workspace_spec: None,
            agent_spec: None,
            agent_progress: None,
        };
        let journal = Journal::open(&path, "http://127.0.0.1:7432", runner).unwrap();
        assert!(Journal::open(&path, "http://127.0.0.1:7432", runner).is_err());
        let mut record = journal.receive(command.clone()).unwrap();
        record.result = Some(OpenCodeStatus::NotConfigured.into());
        journal.save(&record).unwrap();
        drop(journal);
        let journal = Journal::open(&path, "http://127.0.0.1:7432", runner).unwrap();
        let replay = journal.receive(command.clone()).unwrap();
        assert_eq!(replay.ack_request, record.ack_request);
        assert_eq!(replay.result_request, record.result_request);
        assert_eq!(journal.unreported().unwrap().len(), 1);
        let mut changed = command;
        changed.ownership_epoch = 2;
        assert!(journal.receive(changed).is_err());
        record.reported = true;
        journal.save(&record).unwrap();
        assert!(journal.unreported().unwrap().is_empty());
        drop(journal);
        assert!(Journal::open(&path, "http://127.0.0.1:7432", RunnerId::new()).is_err());
    }
}
