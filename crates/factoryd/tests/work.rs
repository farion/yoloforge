use factory_core::*;
use factory_protocol::*;
use factoryd::store::{Store, StoreError};

fn project(store: &Store) -> Project {
    store
        .create_project(CreateProject {
            logo: None,
            runner_mapping: None,
            key: None,
            request_id: RequestId::new(),
            name: "work".into(),
            repository: "/tmp/repo".into(),
            base_branch: "main".into(),
        })
        .unwrap()
}
fn create_ticket(store: &Store, project: ProjectId) -> Ticket {
    store
        .create_ticket(CreateTicket {
            request_id: RequestId::new(),
            project_id: project,
            title: "task".into(),
            description: "".into(),
            acceptance_criteria: "".into(),
            priority: 1,
        })
        .unwrap()
}
fn stream_body(project: ProjectId, runner: RunnerId, ticket: TicketId) -> CreateStream {
    CreateStream {
        request_id: RequestId::new(),
        project_id: project,
        goal: "Implement task".into(),
        runner_id: runner,
        ticket_ids: vec![ticket],
    }
}

#[test]
fn stream_assignment_is_project_scoped_exclusive_and_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let p = project(&store);
    let other = project(&store);
    let t = create_ticket(&store, p.id);
    let runner = RunnerId::new();
    assert!(matches!(
        store.create_stream(stream_body(other.id, runner, t.id)),
        Err(StoreError::Invalid(_))
    ));
    let body = stream_body(p.id, runner, t.id);
    let stream = store.create_stream(body.clone()).unwrap();
    assert_eq!(store.create_stream(body).unwrap(), stream);
    assert!(matches!(
        store.create_stream(stream_body(p.id, runner, t.id)),
        Err(StoreError::WorkConflict)
    ));
    let mut duplicate = stream_body(p.id, runner, t.id);
    duplicate.ticket_ids.push(t.id);
    assert!(matches!(
        store.create_stream(duplicate),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(store.snapshot().unwrap().streams.len(), 1);
}

#[test]
fn accepted_command_survives_restart_results_are_fenced_and_deduplicated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let p = project(&store);
    let t = create_ticket(&store, p.id);
    let runner = RunnerId::new();
    let stream = store
        .create_stream(stream_body(p.id, runner, t.id))
        .unwrap();
    let enqueue = EnqueueCommand {
        prompt: None,
        request_id: RequestId::new(),
        expected_revision: Revision(1),
        kind: CommandKind::CheckOpenCode,
    };
    let cmd = store.enqueue(stream.id, enqueue.clone()).unwrap();
    assert_eq!(store.enqueue(stream.id, enqueue).unwrap(), cmd);
    assert!(store.pending(RunnerId::new()).unwrap().is_empty());
    assert!(matches!(
        store.enqueue(
            stream.id,
            EnqueueCommand {
                prompt: None,
                request_id: RequestId::new(),
                expected_revision: Revision(2),
                kind: CommandKind::CheckOpenCode
            }
        ),
        Err(StoreError::WorkConflict)
    ));
    let ack = AckCommand {
        request_id: RequestId::new(),
        ownership_epoch: 1,
    };
    assert!(matches!(
        store.acknowledge(RunnerId::new(), cmd.id, ack.clone()),
        Err(StoreError::WorkConflict)
    ));
    assert!(matches!(
        store.acknowledge(
            runner,
            cmd.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 2
            }
        ),
        Err(StoreError::WorkConflict)
    ));
    let result = CommandResult {
        request_id: RequestId::new(),
        ownership_epoch: 1,
        result: OpenCodeStatus::Ready {
            version: "2.0.27".into(),
        }
        .into(),
    };
    assert!(matches!(
        store.complete(runner, cmd.id, result.clone()),
        Err(StoreError::WorkConflict)
    ));
    store.acknowledge(runner, cmd.id, ack.clone()).unwrap();
    let cursor = store.snapshot().unwrap().cursor;
    store.acknowledge(runner, cmd.id, ack).unwrap();
    assert_eq!(store.snapshot().unwrap().cursor, cursor);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.pending(runner).unwrap()[0].state,
        CommandState::Accepted
    );
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Running
    );
    let completed = store.complete(runner, cmd.id, result.clone()).unwrap();
    assert_eq!(completed.state, CommandState::Succeeded);
    let cursor = store.snapshot().unwrap().cursor;
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.complete(runner, cmd.id, result).unwrap(), completed);
    assert_eq!(store.snapshot().unwrap().cursor, cursor);
    assert!(store.pending(runner).unwrap().is_empty());
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Idle
    );
    assert_eq!(
        store.snapshot().unwrap().tickets[0].status,
        TicketState::Backlog
    );
    assert!(matches!(
        store.complete(
            runner,
            cmd.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: OpenCodeStatus::Unavailable.into()
            }
        ),
        Err(StoreError::WorkConflict)
    ));
}

#[test]
fn migration_from_v1_preserves_projects_and_adds_work_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(include_str!("../migrations/001_initial.sql"))
        .unwrap();
    let p = Project {
        logo: None,
        runner_mapping: None,
        key: String::new(),
        deleted: false,
        max_sessions: default_session_limit(),
        paused: false,
        runner_bindings: Default::default(),
        mirrors: Default::default(),
        setup: None,
        profile: ExecutionProfile::default(),
        id: ProjectId::new(),
        revision: Revision(1),
        name: "before migration".into(),
        repository: "/tmp/repo".into(),
        base_branch: "main".into(),
        created_unix_ms: 1,
    };
    db.execute(
        "INSERT INTO projects VALUES(?1,?2)",
        rusqlite::params![p.id.to_string(), serde_json::to_string(&p).unwrap()],
    )
    .unwrap();
    drop(db);
    let store = Store::open(&path).unwrap();
    let snap = store.snapshot().unwrap();
    let mut expected=p;
    expected.key="BM".into();
    assert_eq!(snap.projects, vec![expected]);
    assert!(snap.streams.is_empty());
    assert!(snap.commands.is_empty());
}

#[test]
fn workspace_commands_capture_project_metadata_and_require_matching_results() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let p = project(&store);
    let t = create_ticket(&store, p.id);
    let runner = RunnerId::new();
    let stream = store
        .create_stream(stream_body(p.id, runner, t.id))
        .unwrap();
    let cmd = store
        .enqueue(
            stream.id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: Revision(1),
                kind: CommandKind::PrepareWorkspace,
                prompt: None,
            },
        )
        .unwrap();
    let spec = cmd.workspace_spec.as_ref().unwrap();
    assert_eq!(spec.project_id, p.id);
    assert_eq!(spec.branch, stream.branch);
    assert_eq!(spec.base_branch, p.base_branch);
    store
        .acknowledge(
            runner,
            cmd.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
    assert!(matches!(
        store.complete(
            runner,
            cmd.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: OpenCodeStatus::Ready {
                    version: "2.0.27".into()
                }
                .into()
            }
        ),
        Err(StoreError::Invalid(_))
    ));
    let output = CommandOutput::Workspace {
        workspace: Workspace {
            path: "/tmp/managed-worktree".into(),
            branch: spec.branch.clone(),
            base_commit: "a".repeat(40),
        },
    };
    let result = store
        .complete(
            runner,
            cmd.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: output.clone(),
            },
        )
        .unwrap();
    assert_eq!(result.state, CommandState::Succeeded);
    assert_eq!(result.result, Some(output));
}

#[test]
fn agent_profile_is_snapshotted_unknown_keeps_stream_reserved_and_success_requires_terminal_result()
{
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let p = project(&store);
    let t = create_ticket(&store, p.id);
    let runner = RunnerId::new();
    let profile = ExecutionProfile {
        toolchain: "Rust stable".into(),
        test_command: "cargo test".into(),
        provider_id: "openai".into(),
        model_id: "gpt-6-astra".into(),
        ..Default::default()
    };
    let change = UpdateProfile {
        request_id: RequestId::new(),
        expected_revision: Revision(1),
        profile: profile.clone(),
    };
    let configured = store.update_profile(p.id, change.clone()).unwrap();
    assert_eq!(store.update_profile(p.id, change).unwrap(), configured);
    assert!(matches!(
        store.update_profile(
            p.id,
            UpdateProfile {
                request_id: RequestId::new(),
                expected_revision: Revision(1),
                profile: profile.clone()
            }
        ),
        Err(StoreError::RevisionConflict(_))
    ));
    let stream = store
        .create_stream(stream_body(p.id, runner, t.id))
        .unwrap();
    let request = || EnqueueCommand {
        request_id: RequestId::new(),
        expected_revision: Revision(1),
        kind: CommandKind::RunAgent,
        prompt: Some("Implement task".into()),
    };
    assert!(matches!(
        store.enqueue(stream.id, request()),
        Err(StoreError::Invalid(_))
    ));
    let prep = store
        .enqueue(
            stream.id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: Revision(1),
                kind: CommandKind::PrepareWorkspace,
                prompt: None,
            },
        )
        .unwrap();
    store
        .acknowledge(
            runner,
            prep.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
    store
        .complete(
            runner,
            prep.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Workspace {
                    workspace: Workspace {
                        path: "/prepared".into(),
                        branch: stream.branch.clone(),
                        base_commit: "a".repeat(40),
                    },
                },
            },
        )
        .unwrap();
    let mut input = request();
    input.expected_revision = store.snapshot().unwrap().streams[0].revision;
    let cmd = store.enqueue(stream.id, input).unwrap();
    assert_eq!(cmd.agent_spec.as_ref().unwrap().profile, profile);
    store
        .acknowledge(
            runner,
            cmd.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
    let progress = AgentProgress {
        session_id: agent_session_id(cmd.id),
        message_id: agent_message_id(cmd.id),
        phase: AgentPhase::Unknown,
        summary: "uncertain".into(),
    };
    let observation = ObserveAgent {
        ownership_epoch: 1,
        progress: progress.clone(),
    };
    store
        .observe_agent(runner, cmd.id, observation.clone())
        .unwrap();
    let cursor = store.snapshot().unwrap().cursor;
    store.observe_agent(runner, cmd.id, observation).unwrap();
    assert_eq!(store.snapshot().unwrap().cursor, cursor);
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Blocked
    );
    assert_eq!(store.pending(runner).unwrap()[0].id, cmd.id);
    let mut second = request();
    second.expected_revision = store.snapshot().unwrap().streams[0].revision;
    assert!(matches!(
        store.enqueue(stream.id, second),
        Err(StoreError::WorkConflict)
    ));
    assert!(matches!(
        store.complete(
            runner,
            cmd.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Agent {
                    agent: progress.clone()
                }
            }
        ),
        Err(StoreError::WorkConflict)
    ));
    let finalizing = AgentProgress {
        phase: AgentPhase::Finalizing,
        summary: "Saving the session archive".into(),
        ..progress.clone()
    };
    let observed = store
        .observe_agent(runner, cmd.id, ObserveAgent {
            ownership_epoch: 1,
            progress: finalizing.clone(),
        })
        .unwrap();
    assert_eq!(observed.state, CommandState::Accepted);
    assert_eq!(observed.agent_progress, Some(finalizing));
    assert_eq!(store.snapshot().unwrap().streams[0].status, StreamState::Running);
    assert_eq!(store.pending(runner).unwrap()[0].id, cmd.id);

    let final_progress = AgentProgress {
        phase: AgentPhase::Succeeded,
        summary: "Done".into(),
        ..progress
    };
    store
        .complete(
            runner,
            cmd.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Agent {
                    agent: final_progress,
                },
            },
        )
        .unwrap();
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Review
    );
    assert_eq!(
        store.snapshot().unwrap().tickets[0].status,
        TicketState::Backlog
    );
}
