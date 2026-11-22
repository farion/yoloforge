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
    if record.command.kind == CommandKind::PrepareWorkspace {
        let Some(CommandOutput::Workspace { workspace }) = &record.result else {
            anyhow::bail!("Project workspace has not been prepared successfully");
        };
        let effective = crate::setup::effective(args, &record.command)?;
        let registration = effective
            .opencode_registration
            .context("OpenCode registration missing")?;
        let client = OpenCodeClient::from_registration(&registration)?;
        if args.session_readiness {
            let (provider, model) = args
                .inspect_model
                .as_deref()
                .map(|s| {
                    s.split_once('/')
                        .context("Expected provider/model technical IDs")
                })
                .transpose()?
                .unwrap_or(("", ""));
            println!("Recorded workspace: {}", record.command.id);
            println!(
                "{}",
                client.readiness(&workspace.path, provider, model).await?
            );
            return Ok(());
        }
        let status = client
            .project_terminal_command(std::path::Path::new(&workspace.path))
            .await?
            .status()
            .context("Could not launch OpenCode project terminal on runner")?;
        ensure!(status.success(), "OpenCode terminal exited with {status}");
        return Ok(());
    }
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
    if args.session_readiness {
        let profile = &record
            .command
            .agent_spec
            .as_ref()
            .context("Session model profile missing")?
            .profile;
        let (provider, model) = args
            .inspect_model
            .as_deref()
            .map(|s| {
                s.split_once('/')
                    .context("Expected provider/model technical IDs")
            })
            .transpose()?
            .unwrap_or((&profile.provider_id, &profile.model_id));
        println!("Recorded execution: {}", record.command.id);
        println!(
            "{}",
            client
                .readiness(&execution.workspace.path, provider, model)
                .await?
        );
        return Ok(());
    }
    let status = terminal
        .status()
        .context("Could not launch OpenCode terminal on runner")?;
    ensure!(status.success(), "OpenCode terminal exited with {status}");
    Ok(())
}
pub(crate) fn read_record(
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
            && matches!(
                record.command.kind,
                CommandKind::RunAgent
                    | CommandKind::ReviewCheckpoint
                    | CommandKind::PrepareWorkspace
            ),
        "Journal command identity mismatch"
    );
    tx.commit()?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn prepared_project_readiness_needs_no_agent_or_factory_token() {
        use axum::{routing::get, Json, Router};
        use clap::Parser;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let reg = dir.path().join("service.json");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let location = dir.path().to_str().unwrap().to_string();
        let integrations = serde_json::json!({"location":{"directory":location},"data":[]});
        let mcp = integrations.clone();
        let app = Router::new()
            .route(
                "/api/info",
                get(|| async { Json(serde_json::json!({"version":"2.0.27"})) }),
            )
            .route(
                "/api/integration",
                get(move || async move { Json(integrations) }),
            )
            .route("/api/mcp", get(move || async move { Json(mcp) }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        std::fs::write(
            &reg,
            serde_json::json!({"url":endpoint,"password":"fixture"}).to_string(),
        )
        .unwrap();
        let runner = RunnerId::new();
        let id = CommandId::new();
        let journal =
            crate::journal::Journal::open(&path, "http://factory.invalid", runner).unwrap();
        let command:RunnerCommand=serde_json::from_value(serde_json::json!({"id":id,"stream_id":StreamId::new(),"runner_id":runner,"ownership_epoch":1,"kind":"prepare_workspace","state":"pending","created_unix_ms":1})).unwrap();
        let mut record = journal.receive(command).unwrap();
        record.result = Some(CommandOutput::Workspace {
            workspace: Workspace {
                path: location,
                branch: "main".into(),
                base_commit: "a".repeat(40),
            },
        });
        journal.save(&record).unwrap();
        let args = Args::try_parse_from([
            "factory-runner",
            "--id",
            &runner.to_string(),
            "--name",
            "test",
            "--server",
            "http://factory.invalid",
            "--journal",
            path.to_str().unwrap(),
            "--opencode-registration",
            reg.to_str().unwrap(),
            "--open-session",
            &id.to_string(),
            "--session-readiness",
        ])
        .unwrap();
        open(&args, id).await.unwrap();
        assert!(journal.record(id).unwrap().unwrap().agent.is_none());
        server.abort();
    }
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
        let prepared = CommandId::new();
        let command:RunnerCommand=serde_json::from_value(serde_json::json!({"id":prepared,"stream_id":StreamId::new(),"runner_id":runner,"ownership_epoch":1,"kind":"prepare_workspace","state":"pending","created_unix_ms":1})).unwrap();
        journal.receive(command).unwrap();
        let record = read_record(&path, "http://factory", runner, prepared).unwrap();
        assert_eq!(record.command.kind, CommandKind::PrepareWorkspace);
        assert!(record.agent.is_none());
    }
}
