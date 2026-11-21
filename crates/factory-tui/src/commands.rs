use anyhow::Result;
use clap::Subcommand;
use factory_core::*;
use factory_protocol::{client::FactoryClient, *};
mod repository;

#[derive(Subcommand)]
pub enum Command {
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
    /// Request a durable scheduling hold and best-effort interrupt.
    Pause {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        request_id: Option<RequestId>,
    },
    /// Release only a hold confirmed before prompt execution.
    Resume {
        id: StreamId,
        #[arg(long)]
        expected_revision: u64,
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
