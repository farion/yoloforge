use factory_core::*;
use factory_protocol::*;
use factoryd::store::{Store, StoreError};
use serde_json::json;

#[test]
fn supervisor_context_and_extracted_memory_use_the_target_branch_not_its_own_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let target = command(&store);
    finish_agent(&store, &target, "done".into());
    let target_stream = store
        .snapshot()
        .unwrap()
        .streams
        .into_iter()
        .find(|s| s.id == target.stream_id)
        .unwrap();
    for (branch, title) in [
        (target_stream.branch.as_str(), "ONLY_TARGET_RULE"),
        ("other/branch", "FOREIGN_BRANCH_RULE"),
    ] {
        store
            .create_knowledge(CreateKnowledge {
                request_id: RequestId::new(),
                project_id: target_stream.project_id,
                ticket_id: None,
                branch: Some(branch.into()),
                supersedes: None,
                title: title.into(),
                content: "Branch-local reference".into(),
                source: "operator".into(),
                source_command: None,
                priority: 5,
            })
            .unwrap();
    }
    let p = supervisor(&store, &target);
    let e = store
        .supervisor_evaluations(p.project_id)
        .unwrap()
        .pop()
        .unwrap();
    let worker = store
        .acknowledge(
            target.runner_id,
            e.command_id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
    let prompt = &worker.agent_spec.as_ref().unwrap().prompt;
    assert!(prompt.contains("ONLY_TARGET_RULE"));
    assert!(!prompt.contains("FOREIGN_BRANCH_RULE"));
    finish_agent(
        &store,
        &worker,
        json!({"action":"close","reason":"goal met","memory":"Scoped observation"}).to_string(),
    );
    store.supervisor_tick(p.next_unix_ms).unwrap();
    let observation = store
        .knowledge(p.project_id)
        .unwrap()
        .into_iter()
        .find(|n| n.source_command == Some(target.id))
        .unwrap();
    assert_eq!(observation.branch, Some(target_stream.branch.clone()));
    assert!(store
        .knowledge_context(p.project_id, None, 8192)
        .unwrap()
        .notes
        .is_empty());
    assert!(!store
        .knowledge_context_for_branch(p.project_id, None, Some("other/branch"), 8192)
        .unwrap()
        .text
        .contains("Scoped observation"));
}

#[test]
fn migration_retires_unscoped_legacy_supervisor_observations_without_deleting_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let target = command(&store);
    finish_agent(&store, &target, "done".into());
    let p = supervisor(&store, &target);
    let e = store
        .supervisor_evaluations(p.project_id)
        .unwrap()
        .pop()
        .unwrap();
    let worker = store
        .acknowledge(
            target.runner_id,
            e.command_id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
    finish_agent(
        &store,
        &worker,
        json!({"action":"close","reason":"goal met","memory":"A branch-local observation"})
            .to_string(),
    );
    store.supervisor_tick(p.next_unix_ms).unwrap();
    let note = store.knowledge(p.project_id).unwrap().pop().unwrap();
    assert!(note.branch.is_some());
    assert!(store
        .knowledge_context(p.project_id, None, 8192)
        .unwrap()
        .notes
        .is_empty());
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE knowledge SET data=json_remove(data,'$.branch') WHERE id=?1",
        [note.id.to_string()],
    )
    .unwrap();
    db.execute_batch("DROP TABLE scheduler_runs; DROP TABLE scheduler_policies; DROP TABLE reviews; DROP TABLE checkpoint_bundles; DROP TABLE checkpoints; DROP TABLE runtime_bindings; DROP TABLE runtime_terminations; DROP INDEX knowledge_branch_scope; PRAGMA user_version=11;")
        .unwrap();
    drop(db);
    let store = Store::open(&path).unwrap();
    let retired = store.knowledge(p.project_id).unwrap().pop().unwrap();
    assert!(retired.retired);
    assert_eq!(retired.revision, Revision(2));
    assert_eq!(retired.id, note.id);
    assert_eq!(retired.content, note.content);
    assert_eq!(
        store.supervisor_evaluations(p.project_id).unwrap()[0].knowledge_id,
        Some(note.id)
    );
    assert!(store
        .knowledge_context(p.project_id, None, 8192)
        .unwrap()
        .notes
        .is_empty());
    assert!(store
        .events(0, 200)
        .unwrap()
        .events
        .iter()
        .any(|e| e.kind == "knowledge.retired_for_scope_migration"
            && e.aggregate_id == note.id.to_string()));
}

#[tokio::test]
async fn legacy_runner_cannot_poll_or_ack_new_lifecycle_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let first = command(&store);
    finish_agent(&store, &first, "done".into());
    let follow = store
        .continue_agent(
            first.stream_id,
            ContinueAgent {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                previous_command_id: first.id,
                prompt: "follow-up".into(),
            },
        )
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let token = "test-only-lifecycle-capabilities-0123456789";
    let server = tokio::spawn(async move {
        axum::serve(listener, factoryd::router(token, store))
            .await
            .unwrap()
    });
    let http = reqwest::Client::new();
    let mut input = RegisterRunner {
        id: first.runner_id,
        instance_id: InstanceId::new(),
        name: "compatibility".into(),
        location: RunnerLocation::Local,
        version: "0.1.1".into(),
        opencode: OpenCodeStatus::NotConfigured,
        capabilities: vec![],
    };
    let register = || {
        http.post(format!("{base}/v1/runners/register"))
            .bearer_auth(token)
            .header(VERSION_HEADER, "1")
    };
    let registration: Registration = register()
        .json(&input)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let poll = || {
        http.get(format!("{base}/v1/runners/{}/commands", first.runner_id))
            .bearer_auth(token)
            .header(VERSION_HEADER, "1")
            .header(RUNNER_HEADER, registration.registration_id.to_string())
    };
    let pending: Vec<RunnerCommand> = poll()
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(pending.is_empty());
    let response = http
        .post(format!(
            "{base}/v1/runners/{}/commands/{}/ack",
            first.runner_id, follow.id
        ))
        .bearer_auth(token)
        .header(VERSION_HEADER, "1")
        .header(RUNNER_HEADER, registration.registration_id.to_string())
        .json(&AckCommand {
            request_id: RequestId::new(),
            ownership_epoch: 1,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    input.capabilities.push(SESSION_LIFECYCLE_CAPABILITY.into());
    register()
        .json(&input)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let pending: Vec<RunnerCommand> = poll()
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pending, vec![follow]);
    server.abort();
}

fn finish_agent(store: &Store, c: &RunnerCommand, summary: String) {
    store
        .report_archive(
            c.runner_id,
            c.id,
            ReportArchive {
                ownership_epoch: c.ownership_epoch,
                batch: ArchiveBatch {
                    session_id: c.session_id(),
                    revision: 1,
                    captured_unix_ms: 1,
                    final_scan_complete: true,
                    messages: vec![],
                },
            },
        )
        .unwrap();
    store
        .complete(
            c.runner_id,
            c.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: c.ownership_epoch,
                result: CommandOutput::Agent {
                    agent: AgentProgress {
                        session_id: c.session_id(),
                        message_id: agent_message_id(c.id),
                        phase: AgentPhase::Succeeded,
                        summary,
                    },
                },
            },
        )
        .unwrap();
}
#[test]
fn cleanup_after_success_preserves_result_archive_and_unlocks_checkpoint_only_after_proof() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let target = command(&store);
    let binding = RuntimeBinding {
        command_id: target.id,
        stream_id: target.stream_id,
        runner_id: target.runner_id,
        ownership_epoch: 1,
        runtime: RuntimeRef {
            container_id: "a".repeat(64),
            started_at: "2026-11-21T00:00:00Z".into(),
        },
        daemon_id: "test-engine".into(),
        endpoint: "http://127.0.0.1:4096".into(),
        workspace: "/work".into(),
    };
    store
        .report_runtime_binding(target.runner_id, target.id, binding.clone())
        .unwrap();
    finish_agent(&store, &target, "Done, tests passed".into());
    let original = store
        .snapshot()
        .unwrap()
        .commands
        .iter()
        .find(|c| c.id == target.id)
        .unwrap()
        .result
        .clone();
    let revision = || {
        store
            .snapshot()
            .unwrap()
            .streams
            .iter()
            .find(|s| s.id == target.stream_id)
            .unwrap()
            .revision
    };
    let t = store
        .request_termination(
            target.id,
            RequestTermination {
                request_id: RequestId::new(),
                expected_stream_revision: revision(),
                runtime: binding.runtime.clone(),
                previous_request: None,
                reason: "Release completed runtime for checkpoint".into(),
            },
        )
        .unwrap();
    assert!(!t.archive_may_be_incomplete);
    assert_eq!(
        store
            .terminal_cleanup_commands(target.runner_id)
            .unwrap()
            .len(),
        1
    );
    assert!(store
        .enqueue(
            target.stream_id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: revision(),
                kind: CommandKind::Checkpoint,
                prompt: None
            }
        )
        .is_err());
    let report = ReportTermination {
        request_id: t.request_id,
        ownership_epoch: 1,
        state: TerminationState::Stopped,
        proof: Some(RemovalProof {
            runtime: binding.runtime,
            daemon_id: binding.daemon_id,
            observed_unix_ms: 2000,
        }),
    };
    let stopped = store
        .report_termination(target.runner_id, target.id, report.clone())
        .unwrap();
    assert_eq!(
        store
            .report_termination(target.runner_id, target.id, report)
            .unwrap(),
        stopped
    );
    assert_eq!(stopped.result, original);
    assert!(!stopped.archive_may_be_incomplete);
    assert!(store
        .terminal_cleanup_commands(target.runner_id)
        .unwrap()
        .is_empty());
    assert!(
        store
            .archive(target.id, 0, None)
            .unwrap()
            .status
            .unwrap()
            .final_scan_complete
    );
    assert!(store
        .continue_agent(
            target.stream_id,
            ContinueAgent {
                request_id: RequestId::new(),
                expected_revision: revision(),
                previous_command_id: target.id,
                prompt: "Must not reuse removed runtime".into()
            }
        )
        .is_err());
    assert!(store
        .enqueue(
            target.stream_id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: revision(),
                kind: CommandKind::Checkpoint,
                prompt: None
            }
        )
        .is_ok());
}
fn supervisor(store: &Store, target: &RunnerCommand) -> SupervisorPolicy {
    let snapshot = store.snapshot().unwrap();
    let project = snapshot
        .streams
        .iter()
        .find(|s| s.id == target.stream_id)
        .unwrap()
        .project_id;
    let stream = store
        .create_stream(CreateStream {
            request_id: RequestId::new(),
            project_id: project,
            runner_id: target.runner_id,
            goal: "Session supervisor".into(),
            ticket_ids: vec![],
        })
        .unwrap();
    let p = store
        .set_supervisor(
            project,
            SetSupervisor {
                request_id: RequestId::new(),
                expected_revision: Revision(0),
                stream_id: stream.id,
                enabled: true,
                evaluation_budget: 2,
                interval_seconds: 30,
            },
        )
        .unwrap();
    store.supervisor_tick(p.next_unix_ms).unwrap();
    let prepare = store
        .pending(target.runner_id)
        .unwrap()
        .into_iter()
        .find(|c| c.stream_id == stream.id)
        .unwrap();
    assert_eq!(prepare.kind, CommandKind::PrepareWorkspace);
    store
        .acknowledge(
            target.runner_id,
            prepare.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
    store
        .complete(
            target.runner_id,
            prepare.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Workspace {
                    workspace: Workspace {
                        path: "/supervisor".into(),
                        branch: stream.branch,
                        base_commit: "b".repeat(40),
                    },
                },
            },
        )
        .unwrap();
    store.supervisor_tick(p.next_unix_ms).unwrap();
    p
}
#[test]
fn pending_followup_fences_an_already_running_supervisor_close_decision() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let target = command(&store);
    store
        .complete(
            target.runner_id,
            target.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Agent {
                    agent: AgentProgress {
                        session_id: target.session_id(),
                        message_id: agent_message_id(target.id),
                        phase: AgentPhase::Succeeded,
                        summary: "done".into(),
                    },
                },
            },
        )
        .unwrap();
    store
        .report_archive(
            target.runner_id,
            target.id,
            ReportArchive {
                ownership_epoch: 1,
                batch: ArchiveBatch {
                    session_id: target.session_id(),
                    revision: 1,
                    captured_unix_ms: 1,
                    final_scan_complete: true,
                    messages: vec![],
                },
            },
        )
        .unwrap();
    let policy = supervisor(&store, &target);
    let eval = store
        .pending(target.runner_id)
        .unwrap()
        .into_iter()
        .find(|c| c.stream_id == policy.stream_id && c.kind == CommandKind::RunAgent)
        .unwrap();
    store
        .acknowledge(
            target.runner_id,
            eval.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
    store
        .save_task(SaveTask {
            request_id: RequestId::new(),
            id: None,
            expected_revision: Revision(0),
            project_id: policy.project_id,
            prompt: "Continue this work".into(),
            images: vec![],
            requested_session: Some(target.id),
        })
        .unwrap();
    store.complete(target.runner_id,eval.id,CommandResult{request_id:RequestId::new(),ownership_epoch:1,result:CommandOutput::Agent{agent:AgentProgress{session_id:eval.session_id(),message_id:agent_message_id(eval.id),phase:AgentPhase::Succeeded,summary:r#"{"action":"close","reason":"Goal complete","memory":"The previous task completed"}"#.into()}}}).unwrap();
    store.supervisor_tick(policy.next_unix_ms).unwrap();
    assert_eq!(
        store.supervisor_evaluations(policy.project_id).unwrap()[0].outcome,
        "pending_work_preserves_session"
    );
    assert!(store.session_closure(target.id).unwrap().is_none());
}
#[test]
fn autonomous_supervisor_applies_once_with_memory_and_durable_budget() {
    for action in ["compact", "close", "terminate"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let target = command(&store);
        if matches!(action, "terminate" | "close") {
            store
                .report_runtime_binding(
                    target.runner_id,
                    target.id,
                    RuntimeBinding {
                        command_id: target.id,
                        stream_id: target.stream_id,
                        runner_id: target.runner_id,
                        ownership_epoch: 1,
                        runtime: RuntimeRef {
                            container_id: "a".repeat(64),
                            started_at: "2026-11-21T00:00:00Z".into(),
                        },
                        daemon_id: "original-engine".into(),
                        endpoint: "http://127.0.0.1:4096".into(),
                        workspace: "/work/target".into(),
                    },
                )
                .unwrap();
        }
        if action == "close" {
            finish_agent(&store, &target, "Goal reached and tests passed".into());
        } else {
            store
                .observe_agent(
                    target.runner_id,
                    target.id,
                    ObserveAgent {
                        ownership_epoch: 1,
                        progress: AgentProgress {
                            session_id: target.session_id(),
                            message_id: agent_message_id(target.id),
                            phase: AgentPhase::Running,
                            summary: "running".into(),
                        },
                    },
                )
                .unwrap();
        }
        let policy = supervisor(&store, &target);
        let evaluation = store
            .supervisor_evaluations(policy.project_id)
            .unwrap()
            .pop()
            .unwrap();
        let worker = store
            .acknowledge(
                target.runner_id,
                evaluation.command_id,
                AckCommand {
                    request_id: RequestId::new(),
                    ownership_epoch: 1,
                },
            )
            .unwrap();
        assert!(worker.agent_spec.as_ref().unwrap().read_only);
        finish_agent(&store,&worker,json!({"action":action,"reason":"Context and goal assessed","memory":"Unverified observation: tests reported passing; retain the source archive."}).to_string());
        drop(store);
        let store = Store::open(&path).unwrap();
        store.supervisor_tick(policy.next_unix_ms).unwrap();
        store.supervisor_tick(policy.next_unix_ms).unwrap();
        let evaluations = store.supervisor_evaluations(policy.project_id).unwrap();
        assert_eq!(evaluations.len(), 1);
        let applied = &evaluations[0];
        assert!(applied.resolved);
        assert_eq!(applied.outcome, "applied");
        assert!(applied.knowledge_id.is_some());
        assert_eq!(store.knowledge(policy.project_id).unwrap().len(), 1);
        assert_eq!(
            store
                .supervisor_policy(policy.project_id)
                .unwrap()
                .unwrap()
                .remaining_evaluations,
            1
        );
        if action == "terminate" {
            let t = store.termination(target.id).unwrap().unwrap();
            assert_eq!(t.request_id, applied.action_request_id.unwrap());
            assert_eq!(t.state, TerminationState::Requested);
            assert_eq!(t.binding.unwrap().runtime, applied.runtime.clone().unwrap());
            // Autonomous request is not evidence of process termination.
            assert_eq!(
                store
                    .snapshot()
                    .unwrap()
                    .commands
                    .iter()
                    .find(|c| c.id == target.id)
                    .unwrap()
                    .state,
                CommandState::Accepted
            );
        } else if action == "compact" {
            assert_eq!(
                store.compaction(target.id).unwrap().unwrap().request_id,
                applied.action_request_id.unwrap()
            );
        } else {
            assert_eq!(
                store
                    .session_closure(target.id)
                    .unwrap()
                    .unwrap()
                    .request_id,
                applied.action_request_id.unwrap()
            );
            let cleanup = store.termination(target.id).unwrap().unwrap();
            assert_eq!(cleanup.request_id, applied.action_request_id.unwrap());
            assert!(!cleanup.archive_may_be_incomplete);
        }
        let revision = store
            .snapshot()
            .unwrap()
            .streams
            .iter()
            .find(|s| s.id == policy.stream_id)
            .unwrap()
            .revision;
        assert!(store
            .enqueue(
                policy.stream_id,
                EnqueueCommand {
                    request_id: RequestId::new(),
                    expected_revision: revision,
                    kind: CommandKind::RunAgent,
                    prompt: Some("do normal work".into())
                }
            )
            .is_err());
    }
}
#[test]
fn supervisor_discards_decisions_after_policy_change() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let target = command(&store);
    finish_agent(&store, &target, "done".into());
    let p = supervisor(&store, &target);
    let e = store
        .supervisor_evaluations(p.project_id)
        .unwrap()
        .pop()
        .unwrap();
    let worker = store
        .acknowledge(
            target.runner_id,
            e.command_id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
    finish_agent(
        &store,
        &worker,
        json!({"action":"close","reason":"goal met","memory":"Unverified: done"}).to_string(),
    );
    store
        .set_supervisor(
            p.project_id,
            SetSupervisor {
                request_id: RequestId::new(),
                expected_revision: p.revision,
                stream_id: p.stream_id,
                enabled: false,
                evaluation_budget: 0,
                interval_seconds: 30,
            },
        )
        .unwrap();
    store.supervisor_tick(p.next_unix_ms).unwrap();
    assert_eq!(
        store.supervisor_evaluations(p.project_id).unwrap()[0].outcome,
        "policy_changed"
    );
    assert!(store.session_closure(target.id).unwrap().is_none());
    assert!(store.knowledge(p.project_id).unwrap().is_empty());
}

#[test]
fn supervisor_rolls_back_memory_when_final_archive_prevents_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let target = command(&store);
    store
        .observe_agent(
            target.runner_id,
            target.id,
            ObserveAgent {
                ownership_epoch: 1,
                progress: AgentProgress {
                    session_id: target.session_id(),
                    message_id: agent_message_id(target.id),
                    phase: AgentPhase::Running,
                    summary: "running".into(),
                },
            },
        )
        .unwrap();
    let p = supervisor(&store, &target);
    let e = store
        .supervisor_evaluations(p.project_id)
        .unwrap()
        .pop()
        .unwrap();
    let worker = store
        .acknowledge(
            target.runner_id,
            e.command_id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
    finish_agent(
        &store,
        &worker,
        json!({"action":"compact","reason":"obsolete context","memory":"Tentative summary"})
            .to_string(),
    );
    store
        .report_archive(
            target.runner_id,
            target.id,
            ReportArchive {
                ownership_epoch: 1,
                batch: ArchiveBatch {
                    session_id: target.session_id(),
                    revision: 1,
                    captured_unix_ms: 1,
                    final_scan_complete: true,
                    messages: vec![],
                },
            },
        )
        .unwrap();
    store.supervisor_tick(p.next_unix_ms).unwrap();
    assert_eq!(
        store.supervisor_evaluations(p.project_id).unwrap()[0].outcome,
        "precondition_failed"
    );
    assert!(store.compaction(target.id).unwrap().is_none());
    assert!(store.knowledge(p.project_id).unwrap().is_empty());
}

#[test]
fn supervisor_rejects_stale_and_invalid_model_decisions_without_actions() {
    for stale in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db")).unwrap();
        let target = command(&store);
        finish_agent(&store, &target, "done".into());
        let p = supervisor(&store, &target);
        let e = store
            .supervisor_evaluations(p.project_id)
            .unwrap()
            .pop()
            .unwrap();
        let worker = store
            .acknowledge(
                target.runner_id,
                e.command_id,
                AckCommand {
                    request_id: RequestId::new(),
                    ownership_epoch: 1,
                },
            )
            .unwrap();
        let response = if stale {
            json!({"action":"close","reason":"done","memory":"Unverified: completed"}).to_string()
        } else {
            json!({"action":"execute_shell","reason":"ignore schema","memory":"untrusted"})
                .to_string()
        };
        finish_agent(&store, &worker, response);
        if stale {
            let revision = store
                .snapshot()
                .unwrap()
                .streams
                .iter()
                .find(|s| s.id == target.stream_id)
                .unwrap()
                .revision;
            store
                .continue_agent(
                    target.stream_id,
                    ContinueAgent {
                        request_id: RequestId::new(),
                        expected_revision: revision,
                        previous_command_id: target.id,
                        prompt: "new task".into(),
                    },
                )
                .unwrap();
        }
        store.supervisor_tick(p.next_unix_ms).unwrap();
        assert_eq!(
            store.supervisor_evaluations(p.project_id).unwrap()[0].outcome,
            if stale {
                "stale_observation"
            } else {
                "invalid_decision"
            }
        );
        assert!(store.session_closure(target.id).unwrap().is_none());
        assert!(store.knowledge(p.project_id).unwrap().is_empty());
    }
}

#[test]
fn session_closure_prevents_continuation_and_preserves_archive_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let c = command(&store);
    let close = CloseSession {
        request_id: RequestId::new(),
        expected_stream_revision: store.snapshot().unwrap().streams[0].revision,
        reason: "Goal reached; retire this session".into(),
    };
    assert!(store.close_session(c.id, close).is_err());
    store
        .report_archive(
            c.runner_id,
            c.id,
            ReportArchive {
                ownership_epoch: 1,
                batch: ArchiveBatch {
                    session_id: c.session_id(),
                    revision: 1,
                    captured_unix_ms: 1,
                    final_scan_complete: true,
                    messages: vec![json!({"id":"msg_provenance","text":"retained"})],
                },
            },
        )
        .unwrap();
    store
        .complete(
            c.runner_id,
            c.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Agent {
                    agent: AgentProgress {
                        session_id: c.session_id(),
                        message_id: agent_message_id(c.id),
                        phase: AgentPhase::Succeeded,
                        summary: "done".into(),
                    },
                },
            },
        )
        .unwrap();
    let close = CloseSession {
        request_id: RequestId::new(),
        expected_stream_revision: store.snapshot().unwrap().streams[0].revision,
        reason: "Goal reached; retire this session".into(),
    };
    let closure = store.close_session(c.id, close.clone()).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.close_session(c.id, close).unwrap(), closure);
    assert_eq!(store.session_closure(c.id).unwrap(), Some(closure));
    assert_eq!(
        store.archive(c.id, 0, None).unwrap().entries[0].message["text"],
        "retained"
    );
    let revision = store.snapshot().unwrap().streams[0].revision;
    assert!(store
        .continue_agent(
            c.stream_id,
            ContinueAgent {
                request_id: RequestId::new(),
                expected_revision: revision,
                previous_command_id: c.id,
                prompt: "too late".into()
            }
        )
        .is_err());
    let fresh = store
        .enqueue(
            c.stream_id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: revision,
                kind: CommandKind::RunAgent,
                prompt: Some("new session with project knowledge".into()),
            },
        )
        .unwrap();
    assert_ne!(fresh.session_id(), c.session_id());
}

#[test]
fn continuation_keeps_root_session_and_rejects_stale_parent_or_missing_archive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let first = command(&store);
    let result = CommandResult {
        request_id: RequestId::new(),
        ownership_epoch: 1,
        result: CommandOutput::Agent {
            agent: AgentProgress {
                session_id: first.session_id(),
                message_id: agent_message_id(first.id),
                phase: AgentPhase::Succeeded,
                summary: "done".into(),
            },
        },
    };
    store.complete(first.runner_id, first.id, result).unwrap();
    let body = ContinueAgent {
        request_id: RequestId::new(),
        expected_revision: store.snapshot().unwrap().streams[0].revision,
        previous_command_id: first.id,
        prompt: "Use previous context".into(),
    };
    assert!(store.continue_agent(first.stream_id, body.clone()).is_err());
    store
        .report_archive(
            first.runner_id,
            first.id,
            ReportArchive {
                ownership_epoch: 1,
                batch: ArchiveBatch {
                    session_id: first.session_id(),
                    revision: 1,
                    captured_unix_ms: 1,
                    final_scan_complete: true,
                    messages: vec![],
                },
            },
        )
        .unwrap();
    let follow = store.continue_agent(first.stream_id, body.clone()).unwrap();
    assert_eq!(follow.session_id(), first.session_id());
    assert_ne!(follow.id, first.id);
    assert_eq!(
        follow
            .agent_spec
            .as_ref()
            .unwrap()
            .continuation
            .as_ref()
            .unwrap()
            .previous_command_id,
        first.id
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.continue_agent(first.stream_id, body).unwrap(), follow);
    let ack = AckCommand {
        request_id: RequestId::new(),
        ownership_epoch: 1,
    };
    assert!(store
        .acknowledge_supported(follow.runner_id, follow.id, ack.clone(), false)
        .is_err());
    store
        .acknowledge_supported(follow.runner_id, follow.id, ack.clone(), true)
        .unwrap();
    assert!(store
        .acknowledge_supported(follow.runner_id, follow.id, ack, false)
        .is_err());
    let progress = AgentProgress {
        session_id: first.session_id(),
        message_id: agent_message_id(follow.id),
        phase: AgentPhase::Running,
        summary: "follow-up".into(),
    };
    store
        .observe_agent(
            follow.runner_id,
            follow.id,
            ObserveAgent {
                ownership_epoch: 1,
                progress: progress.clone(),
            },
        )
        .unwrap();
    assert!(store
        .observe_agent(
            follow.runner_id,
            follow.id,
            ObserveAgent {
                ownership_epoch: 1,
                progress: AgentProgress {
                    session_id: agent_session_id(follow.id),
                    ..progress.clone()
                }
            }
        )
        .is_err());
    store
        .report_archive(
            follow.runner_id,
            follow.id,
            ReportArchive {
                ownership_epoch: 1,
                batch: ArchiveBatch {
                    session_id: first.session_id(),
                    revision: 1,
                    captured_unix_ms: 2,
                    final_scan_complete: true,
                    messages: vec![],
                },
            },
        )
        .unwrap();
    store
        .complete(
            follow.runner_id,
            follow.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Agent {
                    agent: AgentProgress {
                        phase: AgentPhase::Succeeded,
                        ..progress
                    },
                },
            },
        )
        .unwrap();
    let body = ContinueAgent {
        request_id: RequestId::new(),
        expected_revision: store.snapshot().unwrap().streams[0].revision,
        previous_command_id: first.id,
        prompt: "stale parent".into(),
    };
    assert!(store.continue_agent(first.stream_id, body.clone()).is_err());
    let third = store
        .continue_agent(
            first.stream_id,
            ContinueAgent {
                previous_command_id: follow.id,
                ..body
            },
        )
        .unwrap();
    assert_eq!(third.session_root(), first.id);
}

fn command(store: &Store) -> RunnerCommand {
    let p = store
        .create_project(CreateProject {
            request_id: RequestId::new(),
            name: "archive".into(),
            repository: "/repo".into(),
            base_branch: "main".into(),
        })
        .unwrap();
    store
        .update_profile(
            p.id,
            UpdateProfile {
                request_id: RequestId::new(),
                expected_revision: p.revision,
                profile: ExecutionProfile {
                    provider_id: "openai".into(),
                    model_id: "gpt-6-astra".into(),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    let runner = RunnerId::new();
    let s = store
        .create_stream(CreateStream {
            request_id: RequestId::new(),
            project_id: p.id,
            runner_id: runner,
            goal: "archive".into(),
            ticket_ids: vec![],
        })
        .unwrap();
    let prep = store
        .enqueue(
            s.id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: s.revision,
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
                        branch: s.branch,
                        base_commit: "a".repeat(40),
                    },
                },
            },
        )
        .unwrap();
    let c = store
        .enqueue(
            s.id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                kind: CommandKind::RunAgent,
                prompt: Some("Archive test".into()),
            },
        )
        .unwrap();
    store
        .acknowledge(
            runner,
            c.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap()
}

#[test]
fn compaction_is_durable_revision_fenced_and_blocks_terminal_command_until_resolved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let c = command(&store);
    store
        .observe_agent(
            c.runner_id,
            c.id,
            ObserveAgent {
                ownership_epoch: 1,
                progress: AgentProgress {
                    session_id: agent_session_id(c.id),
                    message_id: agent_message_id(c.id),
                    phase: AgentPhase::Running,
                    summary: "running".into(),
                },
            },
        )
        .unwrap();
    let request = RequestCompaction {
        request_id: RequestId::new(),
        expected_stream_revision: store.snapshot().unwrap().streams[0].revision,
        reason: "Context contains obsolete tool output".into(),
    };
    let compact = store.request_compaction(c.id, request.clone()).unwrap();
    assert_eq!(
        store.request_compaction(c.id, request.clone()).unwrap(),
        compact
    );
    let mut changed = request.clone();
    changed.reason = "different".into();
    assert!(matches!(
        store.request_compaction(c.id, changed),
        Err(StoreError::RequestConflict)
    ));
    let second = RequestCompaction {
        request_id: RequestId::new(),
        expected_stream_revision: store.snapshot().unwrap().streams[0].revision,
        ..request.clone()
    };
    assert!(matches!(
        store.request_compaction(c.id, second),
        Err(StoreError::WorkConflict)
    ));
    let result = CommandResult {
        request_id: RequestId::new(),
        ownership_epoch: 1,
        result: CommandOutput::Agent {
            agent: AgentProgress {
                session_id: agent_session_id(c.id),
                message_id: agent_message_id(c.id),
                phase: AgentPhase::Succeeded,
                summary: "done".into(),
            },
        },
    };
    assert!(matches!(
        store.complete(c.runner_id, c.id, result.clone()),
        Err(StoreError::WorkConflict)
    ));
    let report = ReportCompaction {
        request_id: compact.request_id,
        ownership_epoch: 1,
        state: CompactionState::Admitted,
        summary: "admitted".into(),
    };
    assert!(store
        .report_compaction(RunnerId::new(), c.id, report.clone())
        .is_err());
    store
        .report_compaction(c.runner_id, c.id, report.clone())
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let admitted = store
        .runner_compaction(c.runner_id, c.id, 1)
        .unwrap()
        .unwrap();
    assert_eq!(admitted.state, CompactionState::Admitted);
    assert_eq!(
        admitted.message_id,
        compaction_message_id(compact.request_id)
    );
    assert!(store
        .report_compaction(
            c.runner_id,
            c.id,
            ReportCompaction {
                state: CompactionState::Rejected,
                ..report.clone()
            }
        )
        .is_err());
    let complete = ReportCompaction {
        state: CompactionState::Succeeded,
        summary: "completed message observed".into(),
        ..report
    };
    let done = store
        .report_compaction(c.runner_id, c.id, complete.clone())
        .unwrap();
    assert_eq!(
        store
            .report_compaction(c.runner_id, c.id, complete)
            .unwrap(),
        done
    );
    assert_eq!(
        store.complete(c.runner_id, c.id, result).unwrap().state,
        CommandState::Succeeded
    );
    assert!(store
        .request_compaction(
            c.id,
            RequestCompaction {
                request_id: RequestId::new(),
                expected_stream_revision: store.snapshot().unwrap().streams[0].revision,
                ..request
            }
        )
        .is_err());
}

#[test]
fn archive_preserves_untruncated_versions_reverts_and_fenced_final_scan_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let c = command(&store);
    let text = "🛠".repeat(5000);
    let mut body = ReportArchive {
        ownership_epoch: 1,
        batch: ArchiveBatch {
            session_id: agent_session_id(c.id),
            revision: 1,
            captured_unix_ms: 100,
            final_scan_complete: false,
            messages: vec![json!({"id":"msg_long","type":"user","text":text})],
        },
    };
    let first = store
        .report_archive(c.runner_id, c.id, body.clone())
        .unwrap();
    assert_eq!(
        store
            .report_archive(c.runner_id, c.id, body.clone())
            .unwrap(),
        first
    );
    assert!(matches!(
        store.report_archive(RunnerId::new(), c.id, body.clone()),
        Err(StoreError::WorkConflict)
    ));
    let mut wrong = body.clone();
    wrong.ownership_epoch = 2;
    assert!(store.report_archive(c.runner_id, c.id, wrong).is_err());
    let page = store.archive(c.id, 0, None).unwrap();
    assert_eq!(page.entries[0].message["text"], text);
    let seq = page.next_after;
    body.batch.revision = 2;
    body.batch.messages[0]["text"] = json!("updated");
    store
        .report_archive(c.runner_id, c.id, body.clone())
        .unwrap();
    assert!(store
        .archive(c.id, seq, Some(1))
        .unwrap()
        .entries
        .is_empty());
    body.batch.revision = 3;
    body.batch.messages[0]["text"] = json!(text);
    store
        .report_archive(c.runner_id, c.id, body.clone())
        .unwrap();
    body.batch.revision = 4;
    body.batch.final_scan_complete = true;
    let final_status = store
        .report_archive(c.runner_id, c.id, body.clone())
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .report_archive(c.runner_id, c.id, body.clone())
            .unwrap(),
        final_status
    );
    let mut after = 0;
    let mut messages = Vec::new();
    loop {
        let page = store.archive(c.id, after, None).unwrap();
        assert!(page.status.unwrap().final_scan_complete);
        if page.entries.is_empty() {
            break;
        }
        after = page.next_after;
        messages.extend(page.entries);
    }
    assert_eq!(messages.len(), 3);
    assert_eq!(messages.last().unwrap().message["text"], text);
    body.batch.revision = 5;
    assert!(store.report_archive(c.runner_id, c.id, body).is_err());
}

#[test]
fn archive_rejects_partial_invalid_batches_and_revision_gaps_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let c = command(&store);
    let mut body = ReportArchive {
        ownership_epoch: 1,
        batch: ArchiveBatch {
            session_id: agent_session_id(c.id),
            revision: 1,
            captured_unix_ms: 1,
            final_scan_complete: false,
            messages: vec![
                json!({"id":"valid","text":"keep"}),
                json!({"id":"foreign","sessionID":"wrong"}),
            ],
        },
    };
    assert!(store
        .report_archive(c.runner_id, c.id, body.clone())
        .is_err());
    assert!(store.archive(c.id, 0, None).unwrap().entries.is_empty());
    body.batch.messages.pop();
    body.batch.revision = 2;
    assert!(store
        .report_archive(c.runner_id, c.id, body.clone())
        .is_err());
    body.batch.revision = 1;
    store
        .report_archive(c.runner_id, c.id, body.clone())
        .unwrap();
    body.batch.messages[0]["text"] = json!("changed same revision");
    assert!(store.report_archive(c.runner_id, c.id, body).is_err());
    assert_eq!(
        store.archive(c.id, 0, None).unwrap().entries[0].message["text"],
        "keep"
    );
}
