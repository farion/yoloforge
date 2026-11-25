//! SSH terminal entry. Read the live journal without taking the runner's process lock.
use crate::{journal::Record, Args};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_opencode::OpenCodeClient;
use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;

pub async fn fetch_repository(args: &Args, id: CommandId) -> Result<()> {
    let path = args
        .journal
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!(".factory-data/runner-{}.sqlite3", args.id)));
    let record = read_record(&path, &args.server, args.id, id)?;
    ensure!(
        record.command.kind == CommandKind::PrepareWorkspace,
        "Fetch requires a confirmed workspace preparation"
    );
    let Some(CommandOutput::Workspace { workspace }) = &record.result else {
        anyhow::bail!("Workspace preparation is not complete");
    };
    let directory = std::fs::canonicalize(&workspace.path)?;
    async fn git(path: &std::path::Path, args: &[&str]) -> Result<String> {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(90),
            tokio::process::Command::new("git")
                .current_dir(path)
                .env("GIT_TERMINAL_PROMPT", "0")
                .args(args)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .context("Git fetch timed out; inspect remote refs before retrying")??;
        ensure!(output.status.success(),"Git operation failed; check repository origin and runner Git authentication (output withheld)");
        Ok(String::from_utf8(output.stdout)?.trim().into())
    }
    let root =
        PathBuf::from(git(&directory, &["rev-parse", "--show-toplevel"]).await?).canonicalize()?;
    ensure!(
        root == directory,
        "Recorded workspace is not a repository root"
    );
    ensure!(
        git(&directory, &["symbolic-ref", "--short", "HEAD"]).await? == workspace.branch,
        "Recorded workspace branch changed"
    );
    git(
        &directory,
        &[
            "fetch",
            "--no-prune",
            "--no-prune-tags",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "--refmap=",
            "origin",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    )
    .await?;
    println!("Origin remote-tracking refs fetched for prepared command {id}. Worktree and local branches retained.");
    Ok(())
}

pub async fn open(args: &Args, id: CommandId) -> Result<()> {
    ensure!(args.managed_opencode.is_none() || args.session_readiness,
        "Verwaltetes OpenCode unterstützt hier nur --session-readiness");
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
        (1..=10).contains(&version),
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
    #[tokio::test]
    async fn fetch_uses_recorded_workspace_and_preserves_dirty_index_and_branch() {
        use clap::Parser;
        let dir = tempfile::tempdir().unwrap();
        fn git(path: &std::path::Path, args: &[&str]) -> String {
            let output = std::process::Command::new("git")
                .current_dir(path)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().into()
        }
        let source = dir.path().join("source");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init", "-b", "main"]);
        git(
            &source,
            &["config", "user.email", "fixture@example.invalid"],
        );
        git(&source, &["config", "user.name", "Fixture"]);
        std::fs::write(source.join("file"), "base").unwrap();
        git(&source, &["add", "file"]);
        git(&source, &["commit", "-m", "base"]);
        let work = dir.path().join("work");
        git(
            dir.path(),
            &["clone", source.to_str().unwrap(), work.to_str().unwrap()],
        );
        let head = git(&work, &["rev-parse", "HEAD"]);
        git(
            &work,
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/heads/*",
            ],
        );
        std::fs::write(work.join("file"), "staged").unwrap();
        git(&work, &["add", "file"]);
        std::fs::write(work.join("file"), "dirty").unwrap();
        let index = std::fs::read(work.join(".git/index")).unwrap();
        std::fs::write(source.join("file"), "upstream").unwrap();
        git(&source, &["commit", "-am", "advance"]);
        let upstream = git(&source, &["rev-parse", "HEAD"]);
        let runner = RunnerId::new();
        let id = CommandId::new();
        let path = dir.path().join("journal");
        let journal =
            crate::journal::Journal::open(&path, "http://factory.invalid", runner).unwrap();
        let command:RunnerCommand=serde_json::from_value(serde_json::json!({"id":id,"stream_id":StreamId::new(),"runner_id":runner,"ownership_epoch":1,"kind":"prepare_workspace","state":"accepted","created_unix_ms":1})).unwrap();
        let mut record = journal.receive(command).unwrap();
        record.result = Some(CommandOutput::Workspace {
            workspace: Workspace {
                path: work.to_str().unwrap().into(),
                branch: "main".into(),
                base_commit: head.clone(),
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
            "--fetch-repository",
            &id.to_string(),
        ])
        .unwrap();
        for _ in 0..2 {
            fetch_repository(&args, id).await.unwrap();
        }
        assert_eq!(
            git(&work, &["rev-parse", "refs/remotes/origin/main"]),
            upstream
        );
        assert_eq!(git(&work, &["rev-parse", "HEAD"]), head);
        assert_eq!(std::fs::read(work.join(".git/index")).unwrap(), index);
        assert_eq!(std::fs::read_to_string(work.join("file")).unwrap(), "dirty");
        git(&work, &["symbolic-ref", "HEAD", "refs/heads/changed"]);
        assert!(fetch_repository(&args, id).await.is_err());
    }
}
