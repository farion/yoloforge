//! SSH terminal entry. Read the live journal without taking the runner's process lock.
use crate::{journal::Record, Args};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_opencode::OpenCodeClient;
use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;

pub async fn open(args: &Args, id: CommandId) -> Result<()> {
    let path = args
        .journal
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!(".factory-data/runner-{}.sqlite3", args.id)));
    let record = read_record(&path, &args.server, args.id, id)?;
    let execution = record
        .agent
        .as_ref()
        .context("OpenCode session has not been prepared on this runner")?;
    let effective = crate::setup::effective(args, &record.command)?;
    let registration = effective
        .opencode_registration
        .context("OpenCode registration missing; use --opencode-registration")?;
    let client = OpenCodeClient::from_registration(&registration)?;
    ensure!(
        client.endpoint() == execution.endpoint,
        "Recorded OpenCode endpoint changed"
    );
    let mut terminal = client
        .terminal_command(
            &record.command.session_id(),
            std::path::Path::new(&execution.workspace.path),
        )
        .await?;
    let status = terminal
        .status()
        .context("Could not launch OpenCode terminal on runner")?;
    ensure!(status.success(), "OpenCode terminal exited with {status}");
    Ok(())
}
fn read_record(
    path: &std::path::Path,
    server: &str,
    runner: RunnerId,
    id: CommandId,
) -> Result<Record> {
    let mut db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(std::time::Duration::from_secs(3))?;
    let tx = db.transaction()?;
    let version: u32 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        (1..=9).contains(&version),
        "Unsupported runner journal version"
    );
    let (origin, owner): (String, String) =
        tx.query_row("SELECT server,runner FROM identity WHERE id=1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    ensure!(
        origin == server.trim_end_matches('/') && owner == runner.to_string(),
        "Journal belongs to a different Factory or runner"
    );
    let raw: String = tx.query_row(
        "SELECT data FROM commands WHERE id=?1",
        [id.to_string()],
        |r| r.get(0),
    )?;
    let record: Record = serde_json::from_str(&raw)?;
    ensure!(
        record.command.id == id
            && record.command.runner_id == runner
            && record.command.kind == CommandKind::RunAgent,
        "Journal command identity mismatch"
    );
    tx.commit()?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspection_does_not_take_runner_lock_or_create_missing_journal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let runner = RunnerId::new();
        let id = CommandId::new();
        assert!(read_record(&path, "http://factory", runner, id).is_err());
        assert!(!path.exists());
        let journal = crate::journal::Journal::open(&path, "http://factory", runner).unwrap();
        let command:RunnerCommand=serde_json::from_value(serde_json::json!({"id":id,"stream_id":StreamId::new(),"runner_id":runner,"ownership_epoch":1,"kind":"run_agent","state":"pending","created_unix_ms":1})).unwrap();
        journal.receive(command).unwrap();
        assert_eq!(
            read_record(&path, "http://factory/", runner, id)
                .unwrap()
                .command
                .id,
            id
        );
        assert!(read_record(&path, "http://other", runner, id).is_err());
        assert!(read_record(&path, "http://factory", RunnerId::new(), id).is_err());
    }
}
