use factory_core::*;
use factory_protocol::*;
use factoryd::store::Store;

fn complete(s: &Store, c: &RunnerCommand, result: CommandOutput) {
    s.acknowledge(
        c.runner_id,
        c.id,
        AckCommand {
            request_id: RequestId::new(),
            ownership_epoch: c.ownership_epoch,
        },
    )
    .unwrap();
    s.complete(
        c.runner_id,
        c.id,
        CommandResult {
            request_id: RequestId::new(),
            ownership_epoch: c.ownership_epoch,
            result,
        },
    )
    .unwrap();
}

fn prepared(s: &Store, c: &RunnerCommand) {
    complete(
        s,
        c,
        CommandOutput::Workspace {
            workspace: Workspace {
                path: format!("/work/{}", c.stream_id),
                branch: c.workspace_spec.as_ref().unwrap().branch.clone(),
                base_commit: "a".repeat(40),
            },
        },
    );
}

fn decision(s: &Store, c: &RunnerCommand, json: &str) {
    complete(
        s,
        c,
        CommandOutput::Agent {
            agent: AgentProgress {
                session_id: c.session_id(),
                message_id: agent_message_id(c.id),
                phase: AgentPhase::Succeeded,
                summary: json.into(),
            },
        },
    );
}

fn fixture(path: &std::path::Path) -> (Store, TicketId, RunnerCommand, RunnerCommand) {
    let s = Store::open(path).unwrap();
    let p = s
        .create_project(CreateProject {
            request_id: RequestId::new(),
            name: "Recovery".into(),
            repository: "/repo".into(),
            base_branch: "main".into(),
            key: None,
            logo: None,
            runner_mapping: None,
        })
        .unwrap();
    s.update_profile(
        p.id,
        UpdateProfile {
            request_id: RequestId::new(),
            expected_revision: p.revision,
            profile: ExecutionProfile {
                provider_id: "openai".into(),
                model_id: "test".into(),
                ..Default::default()
            },
        },
    )
    .unwrap();
    let t = s
        .create_ticket(CreateTicket {
            request_id: RequestId::new(),
            project_id: p.id,
            title: "Original task".into(),
            description: "implement".into(),
            acceptance_criteria: "tests".into(),
            priority: 1,
        })
        .unwrap();
    s.set_scheduler(
        p.id,
        SetScheduler {
            request_id: RequestId::new(),
            expected_revision: Revision(0),
            enabled: true,
            runner_ids: vec![RunnerId::new()],
            max_parallel: 1,
        },
    )
    .unwrap();
    s.scheduler_tick().unwrap();
    let c = s
        .snapshot()
        .unwrap()
        .commands
        .into_iter()
        .find(|c| c.kind == CommandKind::PrepareWorkspace)
        .unwrap();
    complete(
        &s,
        &c,
        CommandOutput::WorkspaceError {
            workspace_error: "Git operation failed; inspect repository/worktree before retrying"
                .into(),
        },
    );
    s.scheduler_tick().unwrap();
    let controller = s
        .snapshot()
        .unwrap()
        .commands
        .into_iter()
        .find(|other| other.id != c.id && other.kind == CommandKind::PrepareWorkspace)
        .unwrap();
    prepared(&s, &controller);
    s.scheduler_tick().unwrap();
    let evaluation = s
        .snapshot()
        .unwrap()
        .commands
        .into_iter()
        .find(|c| {
            c.agent_spec.as_ref().is_some_and(|a| {
                a.role == "orchestrator" && a.prompt.contains("recovery orchestrator")
            })
        })
        .unwrap();
    (s, t.id, c, evaluation)
}

#[test]
fn recovery_retries_once_durably_and_resumes_original_task() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (s, tid, failed, evaluation) = fixture(&path);
    decision(
        &s,
        &evaluation,
        r#"{"action":"retry","reason":"A transient repository lock may have cleared"}"#,
    );
    drop(s);
    let s = Store::open(&path).unwrap();
    s.scheduler_tick().unwrap();
    s.scheduler_tick().unwrap();
    let snap = s.snapshot().unwrap();
    let attempts = snap
        .commands
        .iter()
        .filter(|c| c.stream_id == failed.stream_id && c.kind == CommandKind::PrepareWorkspace)
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 2);
    let retry = attempts.into_iter().find(|c| c.id != failed.id).unwrap();
    assert_eq!(retry.workspace_spec, failed.workspace_spec);
    assert_eq!(
        snap.tickets
            .iter()
            .find(|t| t.id == tid)
            .unwrap()
            .execution_command,
        Some(retry.id)
    );
    prepared(&s, retry);
    s.scheduler_tick().unwrap();
    assert!(s
        .snapshot()
        .unwrap()
        .commands
        .iter()
        .any(|c| c.stream_id == failed.stream_id && c.kind == CommandKind::RunAgent));
}

#[test]
fn recovery_commissions_one_scoped_repair_and_does_not_recursively_repair_it() {
    let dir = tempfile::tempdir().unwrap();
    let (s, tid, _, evaluation) = fixture(&dir.path().join("db"));
    decision(
        &s,
        &evaluation,
        r#"{"action":"repair","reason":"The Git diagnostic is missing","prompt":"Find the failing Git step and preserve all existing work."}"#,
    );
    s.scheduler_tick().unwrap();
    s.scheduler_tick().unwrap();
    let snap = s.snapshot().unwrap();
    assert_eq!(snap.tickets.len(), 2);
    let repair = snap.tickets.iter().find(|t| t.id != tid).unwrap();
    assert!(repair.prompt.as_ref().unwrap().contains("Never reset"));
    assert!(repair
        .prompt
        .as_ref()
        .unwrap()
        .contains("Do not implement the original intent"));
    assert!(snap
        .tickets
        .iter()
        .find(|t| t.id == tid)
        .unwrap()
        .routing_note
        .contains(&repair.key));
    // A repair must not spawn an unbounded chain of repair tasks.
    // Resolve normal repair-task routing to a new worker.
    for c in snap
        .commands
        .iter()
        .filter(|c| c.state == CommandState::Pending && c.kind == CommandKind::RunAgent)
    {
        decision(
            &s,
            c,
            r#"{"action":"new","reason":"Dedicated recovery worker"}"#,
        );
    }
    s.scheduler_tick().unwrap();
    let snap = s.snapshot().unwrap();
    let preparation = snap
        .commands
        .iter()
        .find(|c| c.state == CommandState::Pending && c.kind == CommandKind::PrepareWorkspace)
        .unwrap();
    complete(
        &s,
        preparation,
        CommandOutput::WorkspaceError {
            workspace_error: "Still blocked".into(),
        },
    );
    s.scheduler_tick().unwrap();
    assert_eq!(s.snapshot().unwrap().tickets.len(), 2);
    assert!(s
        .snapshot()
        .unwrap()
        .tickets
        .iter()
        .find(|t| t.id == repair.id)
        .unwrap()
        .routing_note
        .contains("Recovery-Grenze"));
}

#[test]
fn changed_intent_discards_pending_recovery_decision() {
    let dir = tempfile::tempdir().unwrap();
    let (s, tid, failed, evaluation) = fixture(&dir.path().join("db"));
    let t = s
        .snapshot()
        .unwrap()
        .tickets
        .into_iter()
        .find(|t| t.id == tid)
        .unwrap();
    s.update_ticket(
        tid,
        UpdateTicket {
            request_id: RequestId::new(),
            expected_revision: t.revision,
            title: Some("Changed by user".into()),
            description: None,
            acceptance_criteria: None,
            priority: None,
            status: None,
        },
    )
    .unwrap();
    decision(&s, &evaluation, r#"{"action":"retry","reason":"Retry"}"#);
    s.scheduler_tick().unwrap();
    assert_eq!(
        s.snapshot()
            .unwrap()
            .commands
            .iter()
            .filter(|c| c.stream_id == failed.stream_id)
            .count(),
        1
    );
}

#[test]
fn completed_repair_retries_original_without_requiring_manual_ticket_completion() {
    let dir = tempfile::tempdir().unwrap();
    let (s, tid, failed, evaluation) = fixture(&dir.path().join("db"));
    decision(
        &s,
        &evaluation,
        r#"{"action":"repair","reason":"Investigate Git failure","prompt":"Diagnose and safely fix the preparation."}"#,
    );
    s.scheduler_tick().unwrap();
    s.scheduler_tick().unwrap();
    let snap = s.snapshot().unwrap();
    let repair = snap.tickets.iter().find(|t| t.id != tid).unwrap().id;
    let routing = snap
        .commands
        .iter()
        .find(|c| c.state == CommandState::Pending && c.kind == CommandKind::RunAgent)
        .unwrap();
    decision(
        &s,
        routing,
        r#"{"action":"new","reason":"Repair in separate session"}"#,
    );
    s.scheduler_tick().unwrap();
    let snap = s.snapshot().unwrap();
    let prepare = snap
        .commands
        .iter()
        .find(|c| c.state == CommandState::Pending && c.kind == CommandKind::PrepareWorkspace)
        .unwrap();
    prepared(&s, prepare);
    s.scheduler_tick().unwrap();
    let snap = s.snapshot().unwrap();
    let worker = snap
        .commands
        .iter()
        .find(|c| c.stream_id == prepare.stream_id && c.kind == CommandKind::RunAgent)
        .unwrap();
    decision(&s, worker, "Verified safe repair completed");
    s.scheduler_tick().unwrap();
    let reviewing = s.snapshot().unwrap();
    assert_eq!(reviewing.tickets.iter().find(|t| t.id == repair).unwrap().status, TicketState::Validating);
    let reviewer = reviewing.commands.iter().find(|c| c.state == CommandState::Pending
        && c.agent_spec.as_ref().is_some_and(|a| a.role == "reviewer")).unwrap();
    decision(&s, reviewer, r#"{"action":"approve"}"#);
    s.scheduler_tick().unwrap();
    let snap = s.snapshot().unwrap();
    assert_eq!(
        snap.tickets.iter().find(|t| t.id == repair).unwrap().status,
        TicketState::Done
    );
    assert_eq!(
        snap.commands
            .iter()
            .filter(|c| c.stream_id == failed.stream_id && c.kind == CommandKind::PrepareWorkspace)
            .count(),
        2
    );
}

#[test]
fn repeated_failures_stop_after_three_orchestrator_evaluations() {
    let dir = tempfile::tempdir().unwrap();
    let (s, tid, failed, mut evaluation) = fixture(&dir.path().join("db"));
    for attempt in 0..3 {
        decision(
            &s,
            &evaluation,
            r#"{"action":"retry","reason":"Bounded guarded retry"}"#,
        );
        s.scheduler_tick().unwrap();
        let snap = s.snapshot().unwrap();
        let retry = snap
            .commands
            .iter()
            .find(|c| c.stream_id == failed.stream_id && c.state == CommandState::Pending)
            .unwrap();
        complete(
            &s,
            retry,
            CommandOutput::WorkspaceError {
                workspace_error: "Still failing".into(),
            },
        );
        s.scheduler_tick().unwrap();
        if attempt < 2 {
            evaluation = s
                .snapshot()
                .unwrap()
                .commands
                .into_iter()
                .find(|c| {
                    c.state == CommandState::Pending
                        && c.agent_spec
                            .as_ref()
                            .is_some_and(|a| a.prompt.contains("recovery orchestrator"))
                })
                .unwrap();
        }
    }
    s.scheduler_tick().unwrap();
    let snap = s.snapshot().unwrap();
    assert_eq!(
        snap.commands
            .iter()
            .filter(|c| c.stream_id == failed.stream_id)
            .count(),
        4
    );
    assert!(snap
        .tickets
        .iter()
        .find(|t| t.id == tid)
        .unwrap()
        .routing_note
        .contains("Recovery-Grenze"));
}

#[test]
fn invalid_model_decision_escalates_without_creating_work() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _, failed, evaluation) = fixture(&dir.path().join("db"));
    decision(
        &s,
        &evaluation,
        r#"{"action":"retry","reason":"Retry","shell":"git reset --hard"}"#,
    );
    s.scheduler_tick().unwrap();
    s.scheduler_tick().unwrap();
    let snap = s.snapshot().unwrap();
    assert_eq!(snap.tickets.len(), 1);
    assert_eq!(
        snap.commands
            .iter()
            .filter(|c| c.stream_id == failed.stream_id)
            .count(),
        1
    );
}

fn failed_git(path: &std::path::Path) -> (Store, Ticket, RunnerCommand) {
    let s = Store::open(path).unwrap();
    let p = s
        .create_project(CreateProject {
            request_id: RequestId::new(),
            name: "Git recovery".into(),
            repository: path.with_file_name("project").display().to_string(),
            base_branch: "main".into(),
            key: None,
            logo: None,
            runner_mapping: None,
        })
        .unwrap();
    s.update_profile(
        p.id,
        UpdateProfile {
            request_id: RequestId::new(),
            expected_revision: p.revision,
            profile: ExecutionProfile {
                provider_id: "openai".into(),
                model_id: "test".into(),
                ..Default::default()
            },
        },
    )
    .unwrap();
    let t = s
        .save_task(SaveTask {
            request_id: RequestId::new(),
            id: None,
            project_id: p.id,
            expected_revision: Revision(0),
            requested_session: None,
            prompt: "commit tag push".into(),
            images: vec![],
        })
        .unwrap();
    let runner = RunnerId::new();
    s.autostart_runner(runner).unwrap();
    s.scheduler_tick().unwrap();
    prepared(&s, &s.pending(runner).unwrap()[0]);
    s.scheduler_tick().unwrap();
    let evaluation = s.pending(runner).unwrap().pop().unwrap();
    decision(
        &s,
        &evaluation,
        r#"{"action":"target_git","reason":"Explicit release","worktree_prompt":null,"git":{"commit":true,"tag":{"kind":"patch"},"push":true}}"#,
    );
    s.scheduler_tick().unwrap();
    let git = s.pending(runner).unwrap().pop().unwrap();
    assert_eq!(git.kind, CommandKind::TargetGit);
    complete(
        &s,
        &git,
        CommandOutput::TargetGit {
            git_succeeded: false,
            git_summary: "Push not confirmed".into(),
        },
    );
    let ticket = s
        .snapshot()
        .unwrap()
        .tickets
        .into_iter()
        .find(|ticket| ticket.id == t.id)
        .unwrap();
    (s, ticket, git)
}

#[test]
fn failed_git_is_evaluated_and_requeued_with_the_same_plan_and_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (s, ticket, failed) = failed_git(&path);
    s.scheduler_tick().unwrap();
    let evaluation = s
        .pending(failed.runner_id)
        .unwrap()
        .into_iter()
        .find(|c| {
            c.agent_spec
                .as_ref()
                .is_some_and(|a| a.prompt.contains("recovery orchestrator"))
        })
        .unwrap();
    assert!(evaluation
        .agent_spec
        .as_ref()
        .unwrap()
        .prompt
        .contains("pins existing commit/tag/destination"));
    decision(
        &s,
        &evaluation,
        r#"{"action":"retry","reason":"Runner will reconcile the pinned execution before retrying the push"}"#,
    );
    drop(s);
    let s = Store::open(&path).unwrap();
    s.scheduler_tick().unwrap();
    s.scheduler_tick().unwrap();
    let retry = s
        .pending(failed.runner_id)
        .unwrap()
        .into_iter()
        .find(|c| c.kind == CommandKind::TargetGit)
        .unwrap();
    assert_ne!(retry.id, failed.id);
    let mut expected = failed.workspace_spec.clone();
    expected
        .as_mut()
        .unwrap()
        .target_git
        .as_mut()
        .unwrap()
        .resume = true;
    assert_eq!(retry.workspace_spec, expected);
    assert_eq!(retry.stream_id, failed.stream_id);
    assert_eq!(retry.ownership_epoch, failed.ownership_epoch);
    assert_eq!(
        s.snapshot()
            .unwrap()
            .tickets
            .iter()
            .find(|t| t.id == ticket.id)
            .unwrap()
            .execution_command,
        Some(retry.id)
    );
    let history = s.task_history(ticket.id, None).unwrap();
    assert!(
        history
            .entries
            .iter()
            .any(|e| e.message == "Push not confirmed"),
        "Keep failed attempt history"
    );
    assert!(history
        .entries
        .iter()
        .any(|e| e.message == "Intent erneut eingereiht"));
}

#[test]
fn deleting_failed_intent_is_fenced_and_preserves_execution_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (store, ticket, failed) = failed_git(&path);
    let body = DeleteFailedTask {
        request_id: RequestId::new(), expected_revision: ticket.revision, failed_command: failed.id,
    };
    assert!(store.delete_failed_task(ticket.id, DeleteFailedTask {
        request_id: RequestId::new(), expected_revision: Revision(ticket.revision.0 - 1), ..body.clone()
    }).is_err());
    assert!(store.delete_failed_task(ticket.id, DeleteFailedTask {
        request_id: RequestId::new(), failed_command: CommandId::new(), ..body.clone()
    }).is_err());
    assert_eq!(store.task_payload(ticket.id).unwrap().id, ticket.id);
    let deleted = store.delete_failed_task(ticket.id, body.clone()).unwrap();
    assert_eq!(deleted.status, TicketState::Cancelled);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.delete_failed_task(ticket.id, body).unwrap().revision, deleted.revision);
    assert!(store.task_payload(ticket.id).is_err());
    assert!(!store.snapshot().unwrap().tickets.iter().any(|t| t.id == ticket.id));
    assert_eq!(store.command_payload(failed.id).unwrap().state, CommandState::Failed);
    assert!(store.task_history(ticket.id, None).unwrap().entries.iter().any(|e| e.message == "Push not confirmed"));
    assert!(store.retry_task(ticket.id, RetryTask {
        request_id: RequestId::new(), expected_revision: deleted.revision, previous_command: failed.id,
    }).is_err());
}

#[test]
fn manual_retry_is_durable_deduplicated_and_not_limited_by_automatic_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (s, mut ticket, mut failed) = failed_git(&path);
    for _ in 0..4 {
        let body = RetryTask {
            request_id: RequestId::new(),
            expected_revision: ticket.revision,
            previous_command: failed.id,
        };
        let next = s.retry_task(ticket.id, body.clone()).unwrap();
        assert_eq!(next.id, ticket.id);
        assert_eq!(next.key, ticket.key);
        assert_eq!(next.prompt, ticket.prompt);
        assert_eq!(
            s.retry_task(ticket.id, body).unwrap().execution_command,
            next.execution_command
        );
        assert!(
            s.retry_task(
                ticket.id,
                RetryTask {
                    request_id: RequestId::new(),
                    expected_revision: ticket.revision,
                    previous_command: failed.id,
                }
            )
            .is_err(),
            "Stale/double clicks cannot duplicate execution"
        );
        failed = s
            .pending(failed.runner_id)
            .unwrap()
            .into_iter()
            .find(|c| Some(c.id) == next.execution_command)
            .unwrap();
        complete(
            &s,
            &failed,
            CommandOutput::TargetGit {
                git_succeeded: false,
                git_summary: "Still blocked".into(),
            },
        );
        ticket = next;
    }
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(
        s.snapshot()
            .unwrap()
            .tickets
            .iter()
            .find(|t| t.id == ticket.id)
            .unwrap()
            .execution_command,
        ticket.execution_command
    );
    assert_eq!(
        s.snapshot()
            .unwrap()
            .commands
            .iter()
            .filter(|c| c.kind == CommandKind::TargetGit)
            .count(),
        5
    );
}

#[test]
fn manual_retry_cannot_duplicate_running_work_or_bypass_project_pause() {
    let dir = tempfile::tempdir().unwrap();
    let (s, ticket, failed) = failed_git(&dir.path().join("db"));
    let project = s.snapshot().unwrap().projects[0].clone();
    s.pause_project(
        project.id,
        PauseProject {
            request_id: RequestId::new(),
            expected_revision: project.revision,
            paused: true,
        },
    )
    .unwrap();
    let body = RetryTask {
        request_id: RequestId::new(),
        expected_revision: ticket.revision,
        previous_command: failed.id,
    };
    assert!(s.retry_task(ticket.id, body).is_err());
    assert!(s.pending(failed.runner_id).unwrap().is_empty());
    assert_eq!(
        s.snapshot()
            .unwrap()
            .tickets
            .iter()
            .find(|t| t.id == ticket.id)
            .unwrap()
            .execution_command,
        Some(failed.id)
    );
}

#[test]
fn manual_retry_wins_over_an_already_pending_automatic_decision() {
    let dir = tempfile::tempdir().unwrap();
    let (s, ticket, failed) = failed_git(&dir.path().join("db"));
    s.scheduler_tick().unwrap();
    let evaluation = s.pending(failed.runner_id).unwrap().pop().unwrap();
    let ticket = s
        .snapshot()
        .unwrap()
        .tickets
        .into_iter()
        .find(|t| t.id == ticket.id)
        .unwrap();
    let requeued = s
        .retry_task(
            ticket.id,
            RetryTask {
                request_id: RequestId::new(),
                expected_revision: ticket.revision,
                previous_command: failed.id,
            },
        )
        .unwrap();
    decision(
        &s,
        &evaluation,
        r#"{"action":"retry","reason":"Late model answer"}"#,
    );
    s.scheduler_tick().unwrap();
    assert_eq!(
        s.snapshot()
            .unwrap()
            .commands
            .iter()
            .filter(|c| c.kind == CommandKind::TargetGit)
            .count(),
        2
    );
    assert_eq!(
        s.pending(failed.runner_id).unwrap()[0].id,
        requeued.execution_command.unwrap()
    );
}

#[test]
fn manual_retry_supports_failed_workspace_and_ordinary_worker_commands() {
    let dir = tempfile::tempdir().unwrap();
    let (s, tid, failed, _) = fixture(&dir.path().join("db"));
    let ticket = s
        .snapshot()
        .unwrap()
        .tickets
        .into_iter()
        .find(|t| t.id == tid)
        .unwrap();
    let requeued = s
        .retry_task(
            tid,
            RetryTask {
                request_id: RequestId::new(),
                expected_revision: ticket.revision,
                previous_command: failed.id,
            },
        )
        .unwrap();
    let prepare = s
        .pending(failed.runner_id)
        .unwrap()
        .into_iter()
        .find(|c| Some(c.id) == requeued.execution_command)
        .unwrap();
    assert_eq!(prepare.kind, CommandKind::PrepareWorkspace);
    prepared(&s, &prepare);
    s.scheduler_tick().unwrap();
    let worker = s
        .pending(failed.runner_id)
        .unwrap()
        .into_iter()
        .find(|c| c.stream_id == failed.stream_id && c.kind == CommandKind::RunAgent)
        .unwrap();
    complete(
        &s,
        &worker,
        CommandOutput::Agent {
            agent: AgentProgress {
                session_id: worker.session_id(),
                message_id: agent_message_id(worker.id),
                phase: AgentPhase::Failed,
                summary: "A tool failed".into(),
            },
        },
    );
    let ticket = s
        .snapshot()
        .unwrap()
        .tickets
        .into_iter()
        .find(|t| t.id == tid)
        .unwrap();
    let requeued = s
        .retry_task(
            tid,
            RetryTask {
                request_id: RequestId::new(),
                expected_revision: ticket.revision,
                previous_command: worker.id,
            },
        )
        .unwrap();
    let retry = s
        .pending(worker.runner_id)
        .unwrap()
        .into_iter()
        .find(|c| Some(c.id) == requeued.execution_command)
        .unwrap();
    assert_eq!(retry.kind, CommandKind::RunAgent);
    assert_eq!(retry.stream_id, worker.stream_id);
    assert!(retry
        .agent_spec
        .unwrap()
        .prompt
        .contains("Inspect existing work first"));
}

#[tokio::test]
async fn retry_endpoint_requires_authentication_and_replays_the_same_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let (s, ticket, failed) = failed_git(&dir.path().join("db"));
    let token = "retry-test-only-0123456789abcdef0123456789";
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let router = factoryd::router(token, s.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
    let body = RetryTask {
        request_id: RequestId::new(), expected_revision: ticket.revision, previous_command: failed.id,
    };
    let response = reqwest::Client::new().post(format!("{url}/v1/tasks/{}/retry", ticket.id))
        .json(&body).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let client = factory_protocol::client::FactoryClient::new(&url, token).unwrap();
    let next = client.retry_task(ticket.id, &body).await.unwrap();
    assert_ne!(next.execution_command, ticket.execution_command);
    assert_eq!(client.retry_task(ticket.id, &body).await.unwrap().execution_command, next.execution_command);
    assert_eq!(s.snapshot().unwrap().commands.iter().filter(|c| c.kind == CommandKind::TargetGit).count(), 2);
    server.abort();
}
