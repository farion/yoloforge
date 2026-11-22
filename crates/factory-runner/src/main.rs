use anyhow::Result;
use clap::{Parser, ValueEnum};
use factory_core::*;
use factory_opencode::OpenCodeClient;
use factory_protocol::{
    client::{ClientError, FactoryClient},
    *,
};
use std::{path::PathBuf, time::Duration};
mod agent;
mod archive;
mod checkpoint;
mod compaction;
mod containment;
mod journal;
mod managed;
mod questions;
mod review;
mod setup;
mod termination;
mod workspace;
use journal::{Journal, Record};

#[derive(Clone, Copy, ValueEnum)]
enum Location {
    Local,
    Server,
}
#[derive(Clone, Parser)]
#[command(
    version,
    about = "Factory runner: durable commands, isolated Git worktrees and OpenCode checks"
)]
struct Args {
    /// Locally installed Docker image used for isolated project review tests (never auto-pulled).
    #[arg(long)]
    review_image: Option<String>,
    #[arg(long, default_value = "/var/run/docker.sock")]
    review_docker_socket: PathBuf,
    /// JSON binding to one dedicated Linux Docker runtime/stream (cgroup v2).
    #[arg(long, requires = "opencode_registration")]
    containment: Option<PathBuf>,
    #[arg(long, default_value = "http://127.0.0.1:7432")]
    server: String,
    #[arg(long)]
    id: RunnerId,
    #[arg(long)]
    name: String,
    #[arg(long, value_enum, default_value = "local")]
    location: Location,
    /// Explicit native OpenCode service registration. Re-read on every heartbeat.
    #[arg(long)]
    opencode_registration: Option<PathBuf>,
    #[arg(long)]
    journal: Option<PathBuf>,
    /// JSON object mapping Factory project IDs to existing local Git repositories.
    #[arg(long)]
    repositories: Option<PathBuf>,
    /// Managed worktrees; defaults to a runner-specific directory beside the journal.
    #[arg(long)]
    workspace_root: Option<PathBuf>,
}

async fn report(
    client: &FactoryClient,
    journal: &Journal,
    registration: RegistrationId,
    mut record: Record,
    args: &Args,
) -> Result<()> {
    let effective = setup::effective(args, &record.command)?;
    let args = &effective;
    if let Some(CommandOutput::Checkpoint { checkpoint }) = &record.result {
        let bytes = tokio::task::block_in_place(|| checkpoint::bundle_bytes(checkpoint))?;
        let receipt = client
            .upload_checkpoint_bundle(
                record.command.runner_id,
                registration,
                record.command.id,
                record.command.ownership_epoch,
                bytes,
            )
            .await?;
        anyhow::ensure!(
            receipt.sha256 == checkpoint.bundle_sha256 && receipt.bytes == checkpoint.bundle_bytes,
            "Factory bundle receipt differs"
        );
    }
    if termination::process(client, journal, &mut record, args, registration).await? {
        return Ok(());
    }
    if containment::control(client, journal, &mut record, args, registration).await? {
        return Ok(());
    }
    if containment::ready(&record, args).await.is_err() {
        client.observe_agent(record.command.runner_id,registration,record.command.id,&ObserveAgent {ownership_epoch:record.command.ownership_epoch,progress:AgentProgress {session_id:record.command.session_id(),message_id:agent_message_id(record.command.id),phase:AgentPhase::Unknown,summary:"Contained runtime identity/state unavailable; local result retained and stream reserved".into()}}).await?;
        return Ok(());
    }
    if !compaction::process(client, journal, &mut record, args, registration, false).await? {
        return Ok(());
    }
    let archived = archive::step(client, journal, &mut record, args, registration).await;
    if !matches!(archived, Ok(true)) {
        client.observe_agent(record.command.runner_id,registration,record.command.id,&ObserveAgent {
            ownership_epoch:record.command.ownership_epoch,
            progress:AgentProgress {session_id:record.command.session_id(),message_id:agent_message_id(record.command.id),phase:AgentPhase::Unknown,summary:"Execution result recorded locally; final session archive pending. Stream remains reserved.".into()},
        }).await?;
        archived?;
        return Ok(());
    }
    sync_excerpt(client, registration, &record).await?;
    let result = record
        .result
        .clone()
        .ok_or_else(|| anyhow::anyhow!("missing journal result"))?;
    let delivered = client
        .command_result(
            record.command.runner_id,
            registration,
            record.command.id,
            &CommandResult {
                request_id: record.result_request,
                ownership_epoch: record.command.ownership_epoch,
                result,
            },
        )
        .await;
    if matches!(
        &delivered,
        Err(ClientError::Http {
            code: Some(ErrorCode::WorkConflict),
            ..
        })
    ) && termination::process(client, journal, &mut record, args, registration).await?
    {
        return Ok(());
    }
    if matches!(
        &delivered,
        Err(ClientError::Http {
            code: Some(ErrorCode::WorkConflict),
            ..
        })
    ) && client
        .runner_compaction(
            record.command.runner_id,
            registration,
            record.command.id,
            record.command.ownership_epoch,
        )
        .await?
        .is_some_and(|r| !r.state.terminal())
    {
        return Ok(());
    }
    delivered?;
    record.reported = true;
    tokio::task::block_in_place(|| journal.save(&record))?;
    println!("Command {} result confirmed", record.command.id);
    Ok(())
}

async fn sync_excerpt(
    client: &FactoryClient,
    registration: RegistrationId,
    record: &Record,
) -> Result<()> {
    if let Some(excerpt) = &record.excerpt {
        client
            .report_excerpt(
                record.command.runner_id,
                registration,
                record.command.id,
                &ReportExcerpt {
                    ownership_epoch: record.command.ownership_epoch,
                    excerpt: excerpt.clone(),
                },
            )
            .await?;
    }
    Ok(())
}

async fn commands(
    client: &FactoryClient,
    journal: &Journal,
    args: &Args,
    registration: RegistrationId,
    next_command: &mut usize,
    next_cleanup: &mut usize,
) -> Result<bool> {
    let cleanups = client
        .terminal_cleanup_commands(args.id, registration)
        .await?;
    if !cleanups.is_empty() {
        let c = cleanups[*next_cleanup % cleanups.len()].clone();
        *next_cleanup = next_cleanup.wrapping_add(1);
        let mut record = tokio::task::block_in_place(|| journal.receive(c))?;
        termination::process(client, journal, &mut record, args, registration).await?;
    }
    let outbox = tokio::task::block_in_place(|| journal.unreported())?;
    for record in outbox {
        report(client, journal, registration, record, args).await?;
    }
    // Process one command per heartbeat; local Git operations have timeouts.
    let pending = client.pending_wait(args.id, registration, 10).await?;
    let selected = if pending.is_empty() {
        None
    } else {
        let index = *next_command % pending.len();
        *next_command = next_command.wrapping_add(1);
        Some(pending[index].clone())
    };
    let had_command = selected.is_some();
    if let Some(command) = selected {
        anyhow::ensure!(
            command.runner_id == args.id,
            "server returned a different runner's command"
        );
        let mut record = tokio::task::block_in_place(|| journal.receive(command))?;
        let effective = setup::effective(args, &record.command)?;
        let args = &effective;
        let ack = client
            .acknowledge(
                args.id,
                registration,
                record.command.id,
                &AckCommand {
                    request_id: record.ack_request,
                    ownership_epoch: record.command.ownership_epoch,
                },
            )
            .await?;
        if matches!(ack.state, CommandState::Succeeded | CommandState::Failed) {
            record.result = ack.result;
            record.reported = true;
            tokio::task::block_in_place(|| journal.save(&record))?;
            return Ok(true);
        }
        record.command = ack;
        tokio::task::block_in_place(|| journal.save(&record))?;
        if termination::process(client, journal, &mut record, args, registration).await? {
            return Ok(true);
        }
        if record.command.kind == CommandKind::RunAgent && record.result.is_none() {
            containment::publish_binding(client, journal, &mut record, registration).await?;
            if containment::control(client, journal, &mut record, args, registration).await? {
                return Ok(true);
            }
            if containment::ready(&record, args).await.is_err() {
                client.observe_agent(args.id,registration,record.command.id,&ObserveAgent {ownership_epoch:record.command.ownership_epoch,progress:AgentProgress {session_id:record.command.session_id(),message_id:agent_message_id(record.command.id),phase:AgentPhase::Unknown,summary:"Contained runtime unavailable, changed or frozen; no external write attempted".into()}}).await?;
                return Ok(true);
            }
            if let Some(control) = client
                .runner_control(
                    args.id,
                    registration,
                    record.command.id,
                    record.command.ownership_epoch,
                )
                .await?
                .filter(|c| c.state != PauseState::Released)
            {
                // A control can arrive between the freezer check above and this read.
                // Contained executions must never fall back to native interrupt.
                if record
                    .agent
                    .as_ref()
                    .is_some_and(|a| a.prompt_attempted && a.runtime.is_some())
                {
                    containment::control(client, journal, &mut record, args, registration).await?;
                    return Ok(true);
                }
                let (state, summary) = agent::pause(journal, &mut record, args).await?;
                compaction::process(client, journal, &mut record, args, registration, false)
                    .await?;
                if agent::capture(&mut record, args).await.is_ok() {
                    tokio::task::block_in_place(|| journal.save(&record))?;
                }
                sync_excerpt(client, registration, &record).await?;
                archive::step(client, journal, &mut record, args, registration).await?;
                client
                    .report_control(
                        args.id,
                        registration,
                        record.command.id,
                        &factory_protocol::ReportControl {
                            proof: None,
                            control_request: control.request_id,
                            ownership_epoch: record.command.ownership_epoch,
                            state,
                            summary,
                        },
                    )
                    .await?;
                return Ok(true);
            }
            let progress = agent::step(journal, &mut record, args).await?;
            let allow_compaction = matches!(
                progress.phase,
                AgentPhase::Running | AgentPhase::WaitingInput
            );
            compaction::process(
                client,
                journal,
                &mut record,
                args,
                registration,
                allow_compaction,
            )
            .await?;
            if agent::capture(&mut record, args).await.is_ok() {
                tokio::task::block_in_place(|| journal.save(&record))?;
            }
            if client
                .runner_control(
                    args.id,
                    registration,
                    record.command.id,
                    record.command.ownership_epoch,
                )
                .await?
                .is_none_or(|c| c.state == PauseState::Released)
            {
                questions::process(client, journal, &mut record, args, registration).await?;
            }
            if matches!(progress.phase, AgentPhase::Succeeded | AgentPhase::Failed) {
                record.result = Some(CommandOutput::Agent { agent: progress });
                tokio::task::block_in_place(|| journal.save(&record))?;
                report(client, journal, registration, record, args).await?;
            } else {
                sync_excerpt(client, registration, &record).await?;
                archive::step(client, journal, &mut record, args, registration).await?;
                client
                    .observe_agent(
                        args.id,
                        registration,
                        record.command.id,
                        &ObserveAgent {
                            ownership_epoch: record.command.ownership_epoch,
                            progress,
                        },
                    )
                    .await?;
            }
            return Ok(true);
        }
        if record.result.is_none() {
            if matches!(
                record.command.kind,
                CommandKind::ReviewCheckpoint | CommandKind::IntegrateReview
            ) {
                let job = client
                    .runner_review_job(
                        args.id,
                        registration,
                        record.command.id,
                        record.command.ownership_epoch,
                    )
                    .await?;
                if let ReviewJob::Review { checkpoint, .. } = &job {
                    let bytes = client
                        .download_checkpoint_bundle(
                            args.id,
                            registration,
                            record.command.id,
                            record.command.ownership_epoch,
                        )
                        .await?;
                    tokio::task::block_in_place(|| {
                        checkpoint::install_bundle(
                            args.workspace_root
                                .as_deref()
                                .expect("workspace root initialized"),
                            checkpoint,
                            &bytes,
                        )
                    })?;
                }
                if let Some(output) = review::step(journal, &mut record, args, &job).await? {
                    record.result = Some(output);
                    tokio::task::block_in_place(|| journal.save(&record))?;
                    report(client, journal, registration, record, args).await?;
                }
                return Ok(true);
            }
            if record.command.kind == CommandKind::PrepareWorkspace {
                if let Some(import) = record
                    .command
                    .workspace_spec
                    .as_ref()
                    .and_then(|s| s.checkpoint.as_ref())
                {
                    let bytes = client
                        .download_checkpoint_bundle(
                            args.id,
                            registration,
                            record.command.id,
                            record.command.ownership_epoch,
                        )
                        .await?;
                    tokio::task::block_in_place(|| {
                        checkpoint::install_bundle(
                            args.workspace_root
                                .as_deref()
                                .expect("workspace root initialized"),
                            import,
                            &bytes,
                        )
                    })?;
                }
            }
            if record.command.kind == CommandKind::Checkpoint {
                let spec = client
                    .runner_checkpoint(
                        args.id,
                        registration,
                        record.command.id,
                        record.command.ownership_epoch,
                    )
                    .await?;
                record.result = Some(checkpoint::execute(journal, &mut record, args, &spec).await);
                tokio::task::block_in_place(|| journal.save(&record))?;
                report(client, journal, registration, record, args).await?;
                return Ok(true);
            }
            record.result = Some(match record.command.kind {
                // Explicitly read-only: safe to repeat after crash before local result commit.
                // Workspace preparation uses its own persisted plan and reconciliation.
                CommandKind::CheckOpenCode => {
                    opencode_status(&args.opencode_registration).await.into()
                }
                CommandKind::PrepareWorkspace => setup::prepare(journal, &mut record, args).await,
                CommandKind::RunAgent => unreachable!("agent execution handled above"),
                CommandKind::Checkpoint => unreachable!("checkpoint execution handled above"),
                CommandKind::ReviewCheckpoint | CommandKind::IntegrateReview => {
                    unreachable!("review handled above")
                }
            });
            tokio::task::block_in_place(|| journal.save(&record))?;
        }
        report(client, journal, registration, record, args).await?;
    }
    Ok(had_command)
}
async fn opencode_status(path: &Option<PathBuf>) -> OpenCodeStatus {
    let Some(path) = path else {
        return OpenCodeStatus::NotConfigured;
    };
    match OpenCodeClient::from_registration(path) {
        Ok(client) => client.status().await,
        Err(_) => OpenCodeStatus::Unavailable,
    }
}

async fn run(mut args: Args) -> Result<()> {
    let client = FactoryClient::from_env(&args.server)?;
    let path = args
        .journal
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!(".factory-data/runner-{}.sqlite3", args.id)));
    let journal = Journal::open(&path, &args.server, args.id)?;
    if args.workspace_root.is_none() {
        args.workspace_root = Some(
            path.parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(format!("workspaces-{}", args.id)),
        );
    }
    let instance_id = InstanceId::new();
    let mut registration: Option<Registration> = None;
    let mut was_connected = false;
    let mut waiting_for_registration = false;
    let mut next_command = 0;
    let mut next_cleanup = 0;
    loop {
        let opencode = opencode_status(&args.opencode_registration).await;
        let result = if let Some(current) = &registration {
            client
                .heartbeat(
                    args.id,
                    &Heartbeat {
                        registration_id: current.registration_id,
                        opencode,
                    },
                )
                .await
        } else {
            client
                .register(&RegisterRunner {
                    capabilities: vec![
                        SESSION_LIFECYCLE_CAPABILITY.into(),
                        CONTAINER_FREEZE_CAPABILITY.into(),
                        RUNTIME_TERMINATION_CAPABILITY.into(),
                        CHECKPOINT_CAPABILITY.into(),
                        REVIEW_CAPABILITY.into(),
                        PROJECT_SETUP_CAPABILITY.into(),
                        TASK_IMAGES_CAPABILITY.into(),
                    ],
                    id: args.id,
                    instance_id,
                    name: args.name.clone(),
                    location: match args.location {
                        Location::Local => RunnerLocation::Local,
                        Location::Server => RunnerLocation::Server,
                    },
                    version: env!("CARGO_PKG_VERSION").into(),
                    opencode,
                })
                .await
        };
        let mut delay = HEARTBEAT_INTERVAL;
        match result {
            Ok(current) => {
                waiting_for_registration = false;
                if !was_connected {
                    println!("Runner {} connected; boot {}", args.id, current.boot_id);
                }
                was_connected = true;
                let command_result = commands(
                    &client,
                    &journal,
                    &args,
                    current.registration_id,
                    &mut next_command,
                    &mut next_cleanup,
                )
                .await;
                registration = Some(current);
                if matches!(command_result, Ok(false)) {
                    delay = Duration::ZERO;
                }
                if let Err(error) = command_result {
                    match error.downcast_ref::<ClientError>() {
                        Some(ClientError::Http {
                            code: Some(ErrorCode::WorkConflict),
                            ..
                        }) => {
                            delay = Duration::from_secs(3);
                        }
                        Some(ClientError::Transport) => {
                            was_connected = false;
                            delay = Duration::from_secs(3);
                        }
                        Some(ClientError::Http { status, .. }) if status.is_server_error() => {
                            was_connected = false;
                            delay = Duration::from_secs(3);
                        }
                        Some(ClientError::Http {
                            code: Some(ErrorCode::NotFound),
                            ..
                        }) => {
                            registration = None;
                            was_connected = false;
                            delay = Duration::from_secs(3);
                        }
                        _ => return Err(error),
                    }
                }
            }
            Err(ClientError::Http {
                code: Some(ErrorCode::RunnerConflict),
                ..
            }) => {
                if !waiting_for_registration {
                    eprintln!("Runner ID still registered by another instance; waiting for its heartbeat lease to expire and retrying automatically (normally up to 60 seconds after shutdown)");
                }
                waiting_for_registration = true;
                registration = None;
                was_connected = false;
                delay = Duration::from_secs(3);
            }
            Err(ClientError::Http {
                code: Some(ErrorCode::NotFound),
                ..
            }) => {
                registration = None;
                was_connected = false;
                delay = Duration::from_millis(100);
            }
            Err(ClientError::Http { status, .. }) if status.is_server_error() => {
                was_connected = false;
                delay = Duration::from_secs(3);
            }
            Err(error @ ClientError::Http { .. }) | Err(error @ ClientError::InvalidResponse) => {
                return Err(error.into())
            }
            Err(ClientError::Transport) => {
                if was_connected || registration.is_none() {
                    eprintln!("Factory unavailable; retrying without starting work");
                }
                was_connected = false;
                delay = Duration::from_secs(3);
            }
        }
        tokio::time::sleep(delay).await;
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = run(args) => result,
        _ = tokio::signal::ctrl_c() => Ok(()),
        _ = terminate.recv() => Ok(()),
    }
}
