use anyhow::Result;
use clap::Subcommand;
use factory_core::*;
use factory_protocol::{client::FactoryClient, *};
pub(crate) mod archive_files;
pub(crate) mod repository;

#[derive(Subcommand)]
pub enum Command {
    Scheduler {
        project: ProjectId,
    },
    SchedulerSet {
        project: ProjectId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        enabled: bool,
        #[arg(long, required = true)]
        runner: Vec<RunnerId>,
        #[arg(long, default_value = "1")]
        max_parallel: u8,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    Review {
        id: CommandId,
    },
    /// Inspect the pinned context and stopped-runtime evidence for a checkpoint command.
    Checkpoint {
        id: CommandId,
    },
    /// Remove a pinned dedicated runtime and fail its active command; volumes/worktree remain.
    Terminate {
        id: CommandId,
        #[arg(long)]
        expected_stream_revision: u64,
        #[arg(long)]
        container_id: String,
        #[arg(long)]
        started_at: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        previous_request: Option<RequestId>,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Inspect a runtime termination, its removal evidence and archive limitation.
    Termination {
        id: CommandId,
    },
    /// Read autonomous session supervisor policy and remaining evaluation budget.
    Supervisor {
        project: ProjectId,
    },
    /// Configure a dedicated supervisor stream (revision 0 creates the policy).
    SupervisorSet {
        project: ProjectId,
        #[arg(long)]
        stream: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long,action=clap::ArgAction::Set)]
        enabled: bool,
        #[arg(long, default_value_t = 10)]
        evaluation_budget: u32,
        #[arg(long, default_value_t = 60)]
        interval_seconds: u32,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Read the latest autonomous evaluations, reasons, source links and action outcomes.
    SupervisorHistory {
        project: ProjectId,
    },
    /// Retire future continuation of a successful archived session, preserving its history.
    CloseSession {
        id: CommandId,
        #[arg(long)]
        expected_stream_revision: u64,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Read the session closure record for any command in its lineage.
    SessionClosure {
        id: CommandId,
    },
    /// Request native compaction for an active agent session (admission is not completion).
    Compact {
        id: CommandId,
        #[arg(long)]
        expected_stream_revision: u64,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Read the latest compaction request and execution state.
    Compaction {
        id: CommandId,
    },
    /// Export persisted message versions as JSONL; creates a new private file.
    Archive {
        id: CommandId,
        #[arg(long)]
        output: std::path::PathBuf,
        /// Extract verified captured binary attachments into a new private directory.
        #[arg(long)]
        attachments: Option<std::path::PathBuf>,
    },
    /// Observed project tokens/cost by model and role; deduplicated archived messages.
    Usage {
        project: ProjectId,
    },
    /// Optional cumulative project token limits (input + output).
    TokenBudget {
        #[command(subcommand)]
        command: TokenBudgetCommand,
    },
    /// Project knowledge with provenance and bounded context selection.
    Knowledge {
        #[command(subcommand)]
        command: KnowledgeCommand,
    },
    /// Set up repositories on THIS machine for a runner.
    Repository {
        #[command(subcommand)]
        command: repository::RepositoryCommand,
    },
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    Question {
        #[command(subcommand)]
        command: QuestionCommand,
    },
    /// Project metadata (does not clone repositories yet).
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    Ticket {
        #[command(subcommand)]
        command: TicketCommand,
    },
    Stream {
        #[command(subcommand)]
        command: StreamCommand,
    },
    /// Durable runner command history.
    Commands {
        #[arg(long)]
        stream: Option<StreamId>,
    },
    /// Read a page of durable events after a cursor.
    Events {
        #[arg(long, default_value_t = 0)]
        after: u64,
    },
}
#[derive(Subcommand)]
pub enum TokenBudgetCommand {
    Show {
        project: ProjectId,
    },
    Set {
        project: ProjectId,
        #[arg(long)]
        expected_revision: u64,
        /// Nonnegative integer or "unlimited". Zero blocks new starts.
        #[arg(long)]
        work: String,
        #[arg(long)]
        orchestration: String,
        #[arg(long)]
        memory: String,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
}
pub(crate) fn token_limit(value: &str) -> anyhow::Result<Option<u64>> {
    if value.trim() == "unlimited" {
        Ok(None)
    } else {
        Ok(Some(value.trim().parse()?))
    }
}
#[derive(Subcommand)]
pub enum KnowledgeCommand {
    /// Read a specific note, including retired content and supersession references.
    Show { id: KnowledgeId },
    Add {
        /// Restrict this note to an exact workstream branch (no inheritance).
        #[arg(long)]
        branch: Option<String>,
        /// Atomically retire and replace a note in the same project/ticket/branch scope.
        #[arg(long, requires = "supersedes_revision")]
        supersedes: Option<KnowledgeId>,
        #[arg(long, requires = "supersedes")]
        supersedes_revision: Option<u64>,
        #[arg(long)]
        project: ProjectId,
        #[arg(long)]
        ticket: Option<TicketId>,
        #[arg(long)]
        title: String,
        #[arg(long)]
        content: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        source_command: Option<CommandId>,
        #[arg(long, default_value_t = 5)]
        priority: u8,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    List {
        #[arg(long)]
        project: ProjectId,
    },
    Context {
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        project: ProjectId,
        #[arg(long)]
        ticket: Option<TicketId>,
        #[arg(long, default_value_t = 8192)]
        budget_bytes: usize,
    },
    Retire {
        id: KnowledgeId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
}
#[derive(Subcommand)]
pub enum DeviceCommand {
    List,
    Revoke {
        id: DeviceId,
    },
    /// Write a short-lived one-use code to a NEW private file (admin only).
    Invite {
        #[arg(long,value_parser=["admin","operator","runner"])]
        role: String,
        #[arg(long)]
        runner: Option<RunnerId>,
        #[arg(long, default_value_t = 600)]
        ttl_seconds: u32,
        #[arg(long)]
        output: std::path::PathBuf,
    },
    /// Redeem a code file and write the device token to a NEW private file.
    Enroll {
        #[arg(long)]
        name: String,
        #[arg(long)]
        code_file: std::path::PathBuf,
        #[arg(long)]
        output: std::path::PathBuf,
    },
}
fn private_file(path: &std::path::Path) -> Result<std::fs::File> {
    Ok(private_options().write(true).create_new(true).open(path)?)
}
fn private_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Windows inherits the ACL of the operator-selected parent directory.
    options
}
#[derive(Subcommand)]
pub enum QuestionCommand {
    List {
        #[arg(long)]
        command: Option<CommandId>,
    },
    Answer {
        id: QuestionId,
        #[arg(long)]
        expected_revision: u64,
        /// Form field answers as a JSON object.
        #[arg(
            long,
            conflicts_with = "decision",
            required_unless_present = "decision"
        )]
        answer_json: Option<String>,
        #[arg(long,value_parser=["once","always","reject"],required_unless_present="answer_json")]
        decision: Option<String>,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
}
#[derive(Subcommand)]
pub enum StreamCommand {
    Review {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        checkpoint: CommandId,
        #[arg(long)]
        expected_target_commit: String,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    Integrate {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        review: CommandId,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Transfer a checkpointed stream to a different runner with a new ownership epoch.
    Handoff {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        checkpoint: CommandId,
        #[arg(long)]
        runner: RunnerId,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Capture Git files and context after confirmed runtime removal (ignored files excluded).
    Checkpoint {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Continue the most recent successful, archived session on its owning runner.
    Continue {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        previous_command: CommandId,
        #[arg(long)]
        prompt: String,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Request a durable scheduling hold and best-effort interrupt.
    Pause {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Release a pre-execution hold or request thaw of a verified dedicated runtime.
    Resume {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Explicitly retry an uncertain container transition with its pinned evidence.
    RetryControl {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        previous_request: RequestId,
        /// JSON file containing the exact persisted proof from snapshot.controls.
        #[arg(long)]
        proof_file: std::path::PathBuf,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Start one OpenCode run in a prepared worktree.
    Run {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        prompt: String,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Prepare an isolated Git worktree on the owning runner.
    Prepare {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    List {
        #[arg(long)]
        project: Option<ProjectId>,
    },
    Create {
        #[arg(long)]
        project: ProjectId,
        #[arg(long)]
        goal: String,
        #[arg(long)]
        runner: RunnerId,
        #[arg(long)]
        ticket: Vec<TicketId>,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Queue a read-only readiness check, not an agent task.
    Check {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
}
#[derive(Subcommand)]
pub enum ProjectCommand {
    /// Edit metadata; repository/base branch are fixed once streams exist.
    Update {
        id: ProjectId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        repository: Option<String>,
        #[arg(long)]
        base_branch: Option<String>,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Replace the project's versioned execution profile.
    Configure {
        id: ProjectId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long, default_value = "")]
        toolchain: String,
        #[arg(long, default_value = "")]
        start_command: String,
        #[arg(long, default_value = "")]
        test_command: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
        #[arg(long, value_parser = ["cl100k_base", "o200k_base", "p50k_base"], requires = "knowledge_tokens")]
        context_tokenizer: Option<String>,
        #[arg(long, requires = "context_tokenizer")]
        knowledge_tokens: Option<usize>,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    List,
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        repository: String,
        #[arg(long, default_value = "main")]
        base_branch: String,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
}
#[derive(Subcommand)]
pub enum TicketCommand {
    Dependencies {
        id: TicketId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        depends_on: Vec<TicketId>,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    List {
        #[arg(long)]
        project: Option<ProjectId>,
    },
    Create {
        #[arg(long)]
        project: ProjectId,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long, default_value = "")]
        acceptance_criteria: String,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(0..=3))]
        priority: u8,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    Update {
        id: TicketId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        acceptance_criteria: Option<String>,
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=3))]
        priority: Option<u8>,
        #[arg(long, value_parser = status)]
        status: Option<TicketState>,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
}
fn status(value: &str) -> Result<TicketState, String> {
    serde_json::from_value(serde_json::Value::String(value.into()))
        .map_err(|_| "Status: backlog, ready, in_progress, blocked, review, done, cancelled".into())
}
fn request(value: Option<RequestId>) -> RequestId {
    let id = value.unwrap_or_default();
    eprintln!("Request-ID: {id} (bei Wiederholung mit --request-id erneut verwenden)");
    id
}
fn print(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
pub async fn execute(client: &FactoryClient, command: Command) -> Result<()> {
    match command {
        Command::Scheduler { project } => print(&client.scheduler_policy(project).await?),
        Command::SchedulerSet {
            project,
            expected_revision,
            enabled,
            runner,
            max_parallel,
            request_id,
        } => print(
            &client
                .set_scheduler(
                    project,
                    &SetScheduler {
                        request_id: request_id.unwrap_or_default(),
                        expected_revision: Revision(expected_revision),
                        enabled,
                        runner_ids: runner,
                        max_parallel,
                    },
                )
                .await?,
        ),
        Command::Ticket {
            command:
                TicketCommand::Dependencies {
                    id,
                    expected_revision,
                    depends_on,
                    request_id,
                },
        } => print(
            &client
                .set_dependencies(
                    id,
                    &SetDependencies {
                        request_id: request_id.unwrap_or_default(),
                        expected_revision: Revision(expected_revision),
                        depends_on,
                    },
                )
                .await?,
        ),
        Command::Review { id } => print(&client.review_job(id).await?),
        Command::Stream {
            command:
                StreamCommand::Review {
                    id,
                    expected_revision,
                    checkpoint,
                    expected_target_commit,
                    request_id,
                },
        } => print(
            &client
                .request_review(
                    id,
                    &RequestReview {
                        request_id: request_id.unwrap_or_default(),
                        expected_revision: Revision(expected_revision),
                        checkpoint_command: checkpoint,
                        expected_target_commit,
                    },
                )
                .await?,
        ),
        Command::Stream {
            command:
                StreamCommand::Integrate {
                    id,
                    expected_revision,
                    review,
                    request_id,
                },
        } => print(
            &client
                .request_integration(
                    id,
                    &RequestIntegration {
                        request_id: request_id.unwrap_or_default(),
                        expected_revision: Revision(expected_revision),
                        review_command: review,
                    },
                )
                .await?,
        ),
        Command::Stream {
            command:
                StreamCommand::Handoff {
                    id,
                    expected_revision,
                    checkpoint,
                    runner,
                    reason,
                    request_id,
                },
        } => print(
            &client
                .handoff(
                    id,
                    &HandoffStream {
                        request_id: request_id.unwrap_or_default(),
                        expected_revision: Revision(expected_revision),
                        checkpoint_command: checkpoint,
                        target_runner: runner,
                        reason,
                    },
                )
                .await?,
        ),
        Command::Stream {
            command:
                StreamCommand::Checkpoint {
                    id,
                    expected_revision,
                    request_id,
                },
        } => print(
            &client
                .enqueue(
                    id,
                    &EnqueueCommand {
                        request_id: request_id.unwrap_or_default(),
                        expected_revision: Revision(expected_revision),
                        kind: CommandKind::Checkpoint,
                        prompt: None,
                    },
                )
                .await?,
        ),
        Command::Checkpoint { id } => print(&client.checkpoint(id).await?),
        Command::Terminate {
            id,
            expected_stream_revision,
            container_id,
            started_at,
            reason,
            previous_request,
            request_id,
        } => print(
            &client
                .request_termination(
                    id,
                    &RequestTermination {
                        request_id: request_id.unwrap_or_default(),
                        expected_stream_revision: Revision(expected_stream_revision),
                        runtime: RuntimeRef {
                            container_id,
                            started_at,
                        },
                        previous_request,
                        reason,
                    },
                )
                .await?,
        ),
        Command::Termination { id } => print(&client.termination(id).await?),
        Command::Supervisor { project } => print(&client.supervisor_policy(project).await?),
        Command::SupervisorHistory { project } => {
            print(&client.supervisor_evaluations(project).await?)
        }
        Command::SupervisorSet {
            project,
            stream,
            expected_revision,
            enabled,
            evaluation_budget,
            interval_seconds,
            request_id,
        } => print(
            &client
                .set_supervisor(
                    project,
                    &SetSupervisor {
                        request_id: request_id.unwrap_or_default(),
                        expected_revision: Revision(expected_revision),
                        stream_id: stream,
                        enabled,
                        evaluation_budget,
                        interval_seconds,
                    },
                )
                .await?,
        ),
        Command::CloseSession {
            id,
            expected_stream_revision,
            reason,
            request_id,
        } => print(
            &client
                .close_session(
                    id,
                    &CloseSession {
                        request_id: request_id.unwrap_or_default(),
                        expected_stream_revision: Revision(expected_stream_revision),
                        reason,
                    },
                )
                .await?,
        ),
        Command::SessionClosure { id } => print(&client.session_closure(id).await?),
        Command::Stream {
            command:
                StreamCommand::Continue {
                    id,
                    expected_revision,
                    previous_command,
                    prompt,
                    request_id,
                },
        } => print(
            &client
                .continue_agent(
                    id,
                    &ContinueAgent {
                        request_id: request_id.unwrap_or_default(),
                        expected_revision: Revision(expected_revision),
                        previous_command_id: previous_command,
                        prompt,
                    },
                )
                .await?,
        ),
        Command::Compact {
            id,
            expected_stream_revision,
            reason,
            request_id,
        } => print(
            &client
                .request_compaction(
                    id,
                    &RequestCompaction {
                        request_id: request_id.unwrap_or_default(),
                        expected_stream_revision: Revision(expected_stream_revision),
                        reason,
                    },
                )
                .await?,
        ),
        Command::Compaction { id } => print(&client.compaction(id).await?),
        Command::Archive {
            id,
            output,
            attachments,
        } => {
            use std::io::Write;
            let mut page = client.archive(id, 0, None).await?;
            let mut file = private_file(&output)?;
            let mut files = attachments
                .as_ref()
                .map(|p| archive_files::Export::new(p))
                .transpose()?;
            let status = page.status.clone();
            let termination = page.termination.clone();
            let through = Some(status.as_ref().map_or(0, |v| v.revision));
            writeln!(
                file,
                "{}",
                serde_json::json!({"command_id":id,"status":status,"termination":termination,"format":"yoloforge-archive-v1"})
            )?;
            let mut after = 0;
            let mut count = 0;
            loop {
                if page.entries.is_empty() {
                    break;
                }
                anyhow::ensure!(page.next_after > after, "Archive cursor did not advance");
                for entry in page.entries {
                    if let Some(files) = &mut files {
                        files.entry(&entry)?;
                    }
                    writeln!(file, "{}", serde_json::to_string(&entry)?)?;
                    count += 1;
                }
                after = page.next_after;
                page = client.archive(id, after, through).await?;
            }
            writeln!(
                file,
                "{}",
                serde_json::json!({"export_complete":true,"versions":count,"through_revision":through})
            )?;
            file.sync_all()?;
            if let Some(files) = &mut files {
                files.supplemental(client, id, through, None).await?;
                files.finish()?;
            }
            print(
                &serde_json::json!({"output":output,"versions":count,"attachments":attachments,"files_extracted":files.as_ref().map(|f|f.count),"files_unavailable":files.as_ref().map(|f|f.unavailable),"final_scan_complete":status.is_some_and(|v|v.final_scan_complete) && termination.as_ref().is_none_or(|t|!t.archive_may_be_incomplete),"termination":termination}),
            )
        }
        Command::Usage { project } => print(&client.project_usage(project).await?),
        Command::TokenBudget { command } => match command {
            TokenBudgetCommand::Show { project } => print(&client.token_budget(project).await?),
            TokenBudgetCommand::Set {
                project,
                expected_revision,
                work,
                orchestration,
                memory,
                request_id,
            } => {
                let body = SetTokenBudget {
                    request_id: request_id.unwrap_or_default(),
                    expected_revision: Revision(expected_revision),
                    limits: TokenLimits {
                        work: token_limit(&work)?,
                        orchestration: token_limit(&orchestration)?,
                        memory: token_limit(&memory)?,
                    },
                };
                print(&client.set_token_budget(project, &body).await?)
            }
        },
        Command::Knowledge { command } => match command {
            KnowledgeCommand::Show { id } => print(&client.knowledge_note(id).await?),
            KnowledgeCommand::Add {
                branch,
                supersedes,
                supersedes_revision,
                project,
                ticket,
                title,
                content,
                source,
                source_command,
                priority,
                request_id,
            } => print(
                &client
                    .create_knowledge(&CreateKnowledge {
                        branch,
                        supersedes: supersedes.zip(supersedes_revision).map(|(id, revision)| {
                            SupersedeKnowledge {
                                id,
                                expected_revision: Revision(revision),
                            }
                        }),
                        request_id: request_id.unwrap_or_default(),
                        project_id: project,
                        ticket_id: ticket,
                        title,
                        content,
                        source,
                        source_command,
                        priority,
                    })
                    .await?,
            ),
            KnowledgeCommand::List { project } => print(&client.knowledge(project).await?),
            KnowledgeCommand::Context {
                branch,
                project,
                ticket,
                budget_bytes,
            } => print(
                &client
                    .knowledge_context_for_branch(project, ticket, branch.as_deref(), budget_bytes)
                    .await?,
            ),
            KnowledgeCommand::Retire {
                id,
                expected_revision,
                request_id,
            } => print(
                &client
                    .retire_knowledge(
                        id,
                        &RetireKnowledge {
                            request_id: request_id.unwrap_or_default(),
                            expected_revision: Revision(expected_revision),
                        },
                    )
                    .await?,
            ),
        },
        Command::Repository { command } => repository::execute(client, command).await,
        Command::Device { command } => {
            use std::io::Write;
            match command {
                DeviceCommand::List => print(&client.devices().await?),
                DeviceCommand::Revoke { id } => print(&client.revoke_device(id).await?),
                DeviceCommand::Invite {
                    role,
                    runner,
                    ttl_seconds,
                    output,
                } => {
                    let mut file = private_file(&output)?;
                    let role = serde_json::from_value(serde_json::Value::String(role))?;
                    let grant = client
                        .enrollment(&CreateEnrollment {
                            role,
                            runner_id: runner,
                            ttl_seconds,
                        })
                        .await?;
                    writeln!(file, "{}", grant.code)?;
                    file.sync_all()?;
                    eprintln!(
                        "Enrollment code saved to {}; expires {} ms UTC",
                        output.display(),
                        grant.expires_unix_ms
                    );
                    Ok(())
                }
                DeviceCommand::Enroll {
                    name,
                    code_file,
                    output,
                } => {
                    let code = std::fs::read_to_string(code_file)?.trim().to_string();
                    let mut file = private_file(&output)?;
                    let credential = client.redeem(&RedeemEnrollment { code, name }).await?;
                    writeln!(file, "{}", credential.token)?;
                    file.sync_all()?;
                    print(&credential.device)
                }
            }
        }
        Command::Project {
            command:
                ProjectCommand::Update {
                    id,
                    expected_revision,
                    name,
                    repository,
                    base_branch,
                    request_id,
                },
        } => print(
            &client
                .update_project(
                    id,
                    &UpdateProject {
                        logo: None,
                        runner_mapping: None,
                        key: None,
                        request_id: request(request_id),
                        expected_revision: Revision(expected_revision),
                        name,
                        repository,
                        base_branch,
                    },
                )
                .await?,
        ),
        Command::Stream {
            command:
                StreamCommand::Pause {
                    id,
                    expected_revision,
                    request_id,
                },
        } => print(
            &client
                .set_control(
                    id,
                    &SetControl {
                        request_id: request(request_id),
                        expected_revision: Revision(expected_revision),
                        action: ControlAction::Pause,
                    },
                )
                .await?,
        ),
        Command::Stream {
            command:
                StreamCommand::RetryControl {
                    id,
                    expected_revision,
                    previous_request,
                    proof_file,
                    reason,
                    request_id,
                },
        } => print(
            &client
                .retry_control(
                    id,
                    &RetryControl {
                        request_id: request_id.unwrap_or_default(),
                        expected_revision: Revision(expected_revision),
                        previous_request,
                        expected_proof: serde_json::from_slice(&std::fs::read(proof_file)?)?,
                        reason,
                    },
                )
                .await?,
        ),
        Command::Stream {
            command:
                StreamCommand::Resume {
                    id,
                    expected_revision,
                    request_id,
                },
        } => print(
            &client
                .set_control(
                    id,
                    &SetControl {
                        request_id: request(request_id),
                        expected_revision: Revision(expected_revision),
                        action: ControlAction::Resume,
                    },
                )
                .await?,
        ),
        Command::Question {
            command: QuestionCommand::List { command },
        } => {
            let values: Vec<_> = client
                .snapshot()
                .await?
                .questions
                .into_iter()
                .filter(|q| command.is_none_or(|id| q.command_id == id))
                .collect();
            print(&values)
        }
        Command::Question {
            command:
                QuestionCommand::Answer {
                    id,
                    expected_revision,
                    answer_json,
                    decision,
                    request_id,
                },
        } => {
            let answer = if let Some(decision) = decision {
                serde_json::json!({"decision":decision})
            } else {
                serde_json::from_str(&answer_json.unwrap_or_default())?
            };
            print(
                &client
                    .answer_question(
                        id,
                        &AnswerQuestion {
                            request_id: request(request_id),
                            expected_revision: Revision(expected_revision),
                            answer,
                        },
                    )
                    .await?,
            )
        }
        Command::Project {
            command:
                ProjectCommand::Configure {
                    id,
                    expected_revision,
                    toolchain,
                    start_command,
                    test_command,
                    provider,
                    model,
                    context_tokenizer,
                    knowledge_tokens,
                    request_id,
                },
        } => print(
            &client
                .update_profile(
                    id,
                    &UpdateProfile {
                        request_id: request(request_id),
                        expected_revision: Revision(expected_revision),
                        profile: ExecutionProfile {
                            context_planning: context_tokenizer
                                .map(|tokenizer| {
                                    Ok::<_, anyhow::Error>(ContextPlanning {
                                        tokenizer: serde_json::from_value(
                                            serde_json::Value::String(tokenizer),
                                        )?,
                                        knowledge_tokens: knowledge_tokens.ok_or_else(|| {
                                            anyhow::anyhow!("Knowledge token budget required")
                                        })?,
                                    })
                                })
                                .transpose()?,
                            toolchain,
                            start_command,
                            test_command,
                            provider_id: provider,
                            model_id: model,
                        },
                    },
                )
                .await?,
        ),
        Command::Stream {
            command:
                StreamCommand::Run {
                    id,
                    expected_revision,
                    prompt,
                    request_id,
                },
        } => print(
            &client
                .enqueue(
                    id,
                    &EnqueueCommand {
                        request_id: request(request_id),
                        expected_revision: Revision(expected_revision),
                        kind: CommandKind::RunAgent,
                        prompt: Some(prompt),
                    },
                )
                .await?,
        ),
        Command::Stream {
            command:
                StreamCommand::Prepare {
                    id,
                    expected_revision,
                    request_id,
                },
        } => print(
            &client
                .enqueue(
                    id,
                    &EnqueueCommand {
                        request_id: request(request_id),
                        expected_revision: Revision(expected_revision),
                        kind: CommandKind::PrepareWorkspace,
                        prompt: None,
                    },
                )
                .await?,
        ),
        Command::Stream {
            command: StreamCommand::List { project },
        } => {
            let values: Vec<_> = client
                .snapshot()
                .await?
                .streams
                .into_iter()
                .filter(|s| project.is_none_or(|p| s.project_id == p))
                .collect();
            print(&values)
        }
        Command::Stream {
            command:
                StreamCommand::Create {
                    project,
                    goal,
                    runner,
                    ticket,
                    request_id,
                },
        } => print(
            &client
                .create_stream(&CreateStream {
                    request_id: request(request_id),
                    project_id: project,
                    goal,
                    runner_id: runner,
                    ticket_ids: ticket,
                })
                .await?,
        ),
        Command::Stream {
            command:
                StreamCommand::Check {
                    id,
                    expected_revision,
                    request_id,
                },
        } => print(
            &client
                .enqueue(
                    id,
                    &EnqueueCommand {
                        request_id: request(request_id),
                        expected_revision: Revision(expected_revision),
                        kind: CommandKind::CheckOpenCode,
                        prompt: None,
                    },
                )
                .await?,
        ),
        Command::Commands { stream } => {
            let values: Vec<_> = client
                .snapshot()
                .await?
                .commands
                .into_iter()
                .filter(|c| stream.is_none_or(|s| c.stream_id == s))
                .collect();
            print(&values)
        }
        Command::Project {
            command: ProjectCommand::List,
        } => print(&client.snapshot().await?.projects),
        Command::Project {
            command:
                ProjectCommand::Create {
                    name,
                    repository,
                    base_branch,
                    request_id,
                },
        } => print(
            &client
                .create_project(&CreateProject {
                    logo: None,
                    runner_mapping: None,
                    key: None,
                    request_id: request(request_id),
                    name,
                    repository,
                    base_branch,
                })
                .await?,
        ),
        Command::Ticket {
            command: TicketCommand::List { project },
        } => {
            let tickets: Vec<_> = client
                .snapshot()
                .await?
                .tickets
                .into_iter()
                .filter(|t| project.is_none_or(|p| p == t.project_id))
                .collect();
            print(&tickets)
        }
        Command::Ticket {
            command:
                TicketCommand::Create {
                    project,
                    title,
                    description,
                    acceptance_criteria,
                    priority,
                    request_id,
                },
        } => print(
            &client
                .create_ticket(&CreateTicket {
                    request_id: request(request_id),
                    project_id: project,
                    title,
                    description,
                    acceptance_criteria,
                    priority,
                })
                .await?,
        ),
        Command::Ticket {
            command:
                TicketCommand::Update {
                    id,
                    expected_revision,
                    title,
                    description,
                    acceptance_criteria,
                    priority,
                    status,
                    request_id,
                },
        } => print(
            &client
                .update_ticket(
                    id,
                    &UpdateTicket {
                        request_id: request(request_id),
                        expected_revision: Revision(expected_revision),
                        title,
                        description,
                        acceptance_criteria,
                        priority,
                        status,
                    },
                )
                .await?,
        ),
        Command::Events { after } => print(&client.events(after).await?),
    }
}
