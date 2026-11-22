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
    for capability in [SESSION_LIFECYCLE_CAPABILITY, CONTEXT_PLANNING_CAPABILITY] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db")).unwrap();
        let first = command(&store);
        finish_agent(&store, &first, "done".into());
        let follow = if capability == CONTEXT_PLANNING_CAPABILITY {
            let snapshot = store.snapshot().unwrap();
            let project = &snapshot.projects[0];
            let mut profile = project.profile.clone();
            profile.context_planning = Some(ContextPlanning {
                tokenizer: ContextTokenizer::O200kBase,
                knowledge_tokens: 128,
            });
            store
                .update_profile(
                    project.id,
                    UpdateProfile {
                        request_id: RequestId::new(),
                        expected_revision: project.revision,
                        profile,
                    },
                )
                .unwrap();
            store
                .enqueue(
                    first.stream_id,
                    EnqueueCommand {
                        request_id: RequestId::new(),
                        expected_revision: snapshot.streams[0].revision,
                        kind: CommandKind::RunAgent,
                        prompt: Some("New task with explicit tokenizer".into()),
                    },
                )
                .unwrap()
        } else {
            store
                .continue_agent(
                    first.stream_id,
                    ContinueAgent {
                        request_id: RequestId::new(),
                        expected_revision: store.snapshot().unwrap().streams[0].revision,
                        previous_command_id: first.id,
                        prompt: "follow-up".into(),
                    },
                )
                .unwrap()
        };
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
        assert_eq!(
            response.status(),
            if capability == CONTEXT_PLANNING_CAPABILITY {
                reqwest::StatusCode::BAD_REQUEST
            } else {
                reqwest::StatusCode::CONFLICT
            }
        );
        input.capabilities.push(capability.into());
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
        let image =
            PromptImage::from_bytes("evidence.png".into(), b"\x89PNG\r\n\x1a\nfixture").unwrap();
        // Seed the persisted target as an image-bearing task; exercise the actual
        // supervisor transaction and subsequent runner acknowledgement.
        let mut persisted_target = target.clone();
        persisted_target.agent_spec.as_mut().unwrap().images = vec![image.clone()];
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute(
                "UPDATE commands SET data=?1 WHERE id=?2",
                rusqlite::params![
                    serde_json::to_string(&persisted_target).unwrap(),
                    target.id.to_string()
                ],
            )
            .unwrap();
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
        assert_eq!(worker.agent_spec.as_ref().unwrap().images, vec![image]);
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
fn supervisor_followup_is_once_in_same_session_and_yields_to_budget_and_interview() {
    for mode in ["continue", "budget", "interview"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let mut target = command(&store);
        if mode == "interview" {
            target.agent_spec.as_mut().unwrap().prompt = "Grill me before implementing".into();
            rusqlite::Connection::open(&path)
                .unwrap()
                .execute(
                    "UPDATE commands SET data=?1 WHERE id=?2",
                    rusqlite::params![
                        serde_json::to_string(&target).unwrap(),
                        target.id.to_string()
                    ],
                )
                .unwrap();
        }
        finish_agent(
            &store,
            &target,
            "Implementation present but requested tests were not run".into(),
        );
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
        if mode == "budget" {
            store
                .set_token_budget(
                    policy.project_id,
                    SetTokenBudget {
                        request_id: RequestId::new(),
                        expected_revision: Revision(0),
                        limits: TokenLimits {
                            work: Some(0),
                            orchestration: None,
                            memory: None,
                        },
                    },
                )
                .unwrap();
        }
        finish_agent(&store,&worker,json!({"action":"continue","reason":"Requested tests are missing","memory":"Tests are still unverified","followup":"Run the originally requested tests and fix failures."}).to_string());
        store.supervisor_tick(policy.next_unix_ms + 1).unwrap();
        let resolved = store
            .supervisor_evaluations(policy.project_id)
            .unwrap()
            .pop()
            .unwrap();
        if mode == "continue" {
            assert_eq!(resolved.outcome, "applied");
            let id = resolved.followup_command_id.unwrap();
            let next = store
                .pending(target.runner_id)
                .unwrap()
                .into_iter()
                .find(|c| c.id == id)
                .unwrap();
            assert_eq!(next.session_id(), target.session_id());
            assert_eq!(next.stream_id, target.stream_id);
            assert_eq!(
                next.agent_spec
                    .as_ref()
                    .unwrap()
                    .continuation
                    .as_ref()
                    .unwrap()
                    .previous_command_id,
                target.id
            );
            assert!(!next.agent_spec.as_ref().unwrap().read_only);
            assert!(resolved.knowledge_id.is_some());
            drop(store);
            let store = Store::open(&path).unwrap();
            store.supervisor_tick(policy.next_unix_ms + 2).unwrap();
            assert_eq!(
                store
                    .snapshot()
                    .unwrap()
                    .commands
                    .iter()
                    .filter(|c| c.agent_spec.as_ref().is_some_and(|s| s
                        .continuation
                        .as_ref()
                        .is_some_and(|v| v.previous_command_id == target.id)))
                    .count(),
                1
            );
        } else {
            assert_eq!(
                resolved.outcome,
                if mode == "budget" {
                    "precondition_failed"
                } else {
                    "human_interview_preserves_session"
                }
            );
            assert!(resolved.followup_command_id.is_none() && resolved.knowledge_id.is_none());
        }
    }
}

#[test]
fn reallocation_requires_stopped_runtime_and_preserves_fixed_session_and_task_provenance() {
    for mode in ["stopped", "unverified", "fixed"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let target = command(&store);
        let owner = store
            .snapshot()
            .unwrap()
            .streams
            .into_iter()
            .find(|s| s.id == target.stream_id)
            .unwrap();
        let ticket = store
            .save_task(SaveTask {
                request_id: RequestId::new(),
                id: None,
                expected_revision: Revision(0),
                project_id: owner.project_id,
                prompt: "Complete the original task".into(),
                images: vec![],
                requested_session: None,
            })
            .unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute("UPDATE tickets SET data=json_set(data,'$.execution_command',?2,'$.status','in_progress') WHERE id=?1",rusqlite::params![ticket.id.to_string(),target.id.to_string()]).unwrap();
        db.execute(
            "INSERT INTO stream_tickets(ticket_id,stream_id) VALUES(?1,?2)",
            rusqlite::params![ticket.id.to_string(), target.stream_id.to_string()],
        )
        .unwrap();
        let runtime = RuntimeRef {
            container_id: "e".repeat(64),
            started_at: "fixture".into(),
        };
        store
            .report_runtime_binding(
                target.runner_id,
                target.id,
                RuntimeBinding {
                    command_id: target.id,
                    stream_id: target.stream_id,
                    runner_id: target.runner_id,
                    ownership_epoch: 1,
                    runtime: runtime.clone(),
                    daemon_id: "fixture-daemon".into(),
                    endpoint: "http://127.0.0.1:4096".into(),
                    workspace: "/work".into(),
                },
            )
            .unwrap();
        if mode != "unverified" {
            let termination = store
                .request_termination(
                    target.id,
                    RequestTermination {
                        request_id: RequestId::new(),
                        expected_stream_revision: owner.revision,
                        runtime: runtime.clone(),
                        previous_request: None,
                        reason: "Unrecoverable attempt; preserve work for reassessment".into(),
                    },
                )
                .unwrap();
            store
                .report_termination(
                    target.runner_id,
                    target.id,
                    ReportTermination {
                        request_id: termination.request_id,
                        ownership_epoch: 1,
                        state: TerminationState::Stopped,
                        proof: Some(RemovalProof {
                            runtime: runtime.clone(),
                            daemon_id: "fixture-daemon".into(),
                            observed_unix_ms: 1,
                        }),
                    },
                )
                .unwrap();
        }
        let policy = supervisor(&store, &target);
        let evaluation = store
            .supervisor_evaluations(owner.project_id)
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
        if mode == "fixed" {
            db.execute(
                "UPDATE tickets SET data=json_set(data,'$.requested_session',?2) WHERE id=?1",
                rusqlite::params![ticket.id.to_string(), target.id.to_string()],
            )
            .unwrap();
        }
        finish_agent(&store,&worker,json!({"action":"reallocate","reason":"Original task still needs work in an eligible session","memory":"Prior work remains in the stopped worktree; inspect retained evidence before redoing it."}).to_string());
        store.supervisor_tick(policy.next_unix_ms + 1).unwrap();
        let result = store
            .supervisor_evaluations(owner.project_id)
            .unwrap()
            .pop()
            .unwrap();
        let task = store
            .snapshot()
            .unwrap()
            .tickets
            .into_iter()
            .find(|t| t.id == ticket.id)
            .unwrap();
        assert_eq!(task.prompt, ticket.prompt);
        assert_eq!(task.images, ticket.images);
        if mode == "stopped" {
            assert_eq!(result.outcome, "applied");
            assert_eq!(task.status, TicketState::Ready);
            assert!(task.execution_command.is_none());
            assert!(store
                .knowledge_context(owner.project_id, Some(ticket.id), 8192)
                .unwrap()
                .text
                .contains(&target.id.to_string()));
            store
                .create_stream(CreateStream {
                    request_id: RequestId::new(),
                    project_id: owner.project_id,
                    runner_id: target.runner_id,
                    goal: "New allocation".into(),
                    ticket_ids: vec![ticket.id],
                })
                .unwrap();
            drop(store);
            let store = Store::open(&path).unwrap();
            store.supervisor_tick(policy.next_unix_ms + 2).unwrap();
            assert_eq!(
                store
                    .events(0, 200)
                    .unwrap()
                    .events
                    .iter()
                    .filter(|e| e.kind == "task.reallocated")
                    .count(),
                1
            );
        } else {
            assert_eq!(result.outcome, "inapplicable");
            assert_eq!(task.execution_command, Some(target.id));
            assert!(result.knowledge_id.is_none());
        }
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

#[test]
fn usage_deduplicates_continuation_snapshots_and_preserves_unknown_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("usage.db");
    let store = Store::open(&path).unwrap();
    let first = command(&store);
    let project = store.snapshot().unwrap().projects[0].id;
    let message = |input, cost| {
        serde_json::json!({"id":"msg_usage","type":"assistant",
        "model":{"providerID":"openai","id":"test"},"time":{"completed":1},
        "tokens":{"input":input,"output":5,"reasoning":2,"cache":{"read":3,"write":0}},"cost":cost})
    };
    let batch = |revision, messages, final_scan_complete| ReportArchive {
        ownership_epoch: 1,
        batch: ArchiveBatch {
            session_id: first.session_id(),
            revision,
            captured_unix_ms: revision,
            final_scan_complete,
            messages,
        },
    };
    let initial = batch(1, vec![message(10, 0.1)], false);
    store
        .report_archive(first.runner_id, first.id, initial.clone())
        .unwrap();
    store
        .report_archive(first.runner_id, first.id, initial)
        .unwrap();
    store
        .report_archive(
            first.runner_id,
            first.id,
            batch(2, vec![message(12, 0.2)], true),
        )
        .unwrap();
    store
        .complete(
            first.runner_id,
            first.id,
            CommandResult {
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
            },
        )
        .unwrap();
    let next = store
        .continue_agent(
            first.stream_id,
            ContinueAgent {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                previous_command_id: first.id,
                prompt: "next".into(),
            },
        )
        .unwrap();
    let budget_request = SetTokenBudget {
        request_id: RequestId::new(),
        expected_revision: Revision(0),
        limits: TokenLimits {
            work: Some(17),
            orchestration: None,
            memory: None,
        },
    };
    let budget = store
        .set_token_budget(project, budget_request.clone())
        .unwrap();
    assert_eq!(
        store.set_token_budget(project, budget_request).unwrap(),
        budget
    );
    assert!(
        store
            .acknowledge_supported(
                next.runner_id,
                next.id,
                AckCommand {
                    request_id: RequestId::new(),
                    ownership_epoch: 1
                },
                true
            )
            .is_err(),
        "Pending starts must recheck the latest budget"
    );
    store
        .set_token_budget(
            project,
            SetTokenBudget {
                request_id: RequestId::new(),
                expected_revision: budget.revision,
                limits: TokenLimits {
                    work: Some(18),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    store
        .acknowledge_supported(
            next.runner_id,
            next.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
            true,
        )
        .unwrap();
    store.report_archive(next.runner_id,next.id,batch(1,vec![message(12,0.2),serde_json::json!({
        "id":"msg_unknown_usage","type":"assistant","model":{"providerID":"openai","id":"test"},"tokens":-1,"cost":"unknown"
    })],false)).unwrap();
    let report = store.project_usage(project).unwrap();
    let budget = store.token_budget(project).unwrap();
    store
        .set_token_budget(
            project,
            SetTokenBudget {
                request_id: RequestId::new(),
                expected_revision: budget.revision,
                limits: TokenLimits {
                    work: Some(0),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    assert_eq!(
        store
            .acknowledge_supported(
                next.runner_id,
                next.id,
                AckCommand {
                    request_id: RequestId::new(),
                    ownership_epoch: 1
                },
                true
            )
            .unwrap()
            .state,
        CommandState::Accepted,
        "Lowering limits must not revoke an accepted command"
    );
    let report = ProjectUsage {
        token_budget: Some(store.token_budget(project).unwrap()),
        ..report
    };
    assert_eq!(report.groups.len(), 1);
    let group = &report.groups[0];
    assert_eq!(group.assistant_messages, 2);
    assert_eq!(group.unfinished_messages, 1);
    assert_eq!(group.messages_missing_tokens, 1);
    assert_eq!(group.messages_missing_cost, 1);
    assert_eq!(group.tokens.input, 12);
    assert_eq!(group.tokens.output, 5);
    assert_eq!(group.tokens.reasoning, 2);
    assert_eq!(group.tokens.cache.read, 3);
    assert_eq!(group.reported_cost_usd, 0.2);
    assert_eq!(group.model_id.as_deref(), Some("test"));
    assert_eq!(group.role, "work");
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.project_usage(project).unwrap(), report);
    assert!(store.project_usage(ProjectId::new()).is_err());
}

#[test]
fn interrupted_native_session_requires_confirmation_archive_and_explicit_release() {
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
                phase: AgentPhase::Interrupted,
                summary: "Native interruption, not process termination".into(),
            },
        },
    };
    assert!(store
        .complete(first.runner_id, first.id, result.clone())
        .is_err());
    let control = store
        .set_control(
            first.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Pause,
            },
        )
        .unwrap();
    store
        .report_control(
            first.runner_id,
            first.id,
            ReportControl {
                ownership_epoch: 1,
                control_request: control.request_id,
                proof: None,
                state: PauseState::SessionInterrupted,
                summary: "Native session is inactive and interrupted".into(),
            },
        )
        .unwrap();
    assert!(
        store
            .set_control(
                first.stream_id,
                SetControl {
                    request_id: RequestId::new(),
                    expected_revision: store.snapshot().unwrap().streams[0].revision,
                    action: ControlAction::Resume,
                }
            )
            .is_err(),
        "Cannot release before command finalization"
    );
    let completed = store.complete(first.runner_id, first.id, result).unwrap();
    assert_eq!(
        completed.state,
        CommandState::Failed,
        "Interrupted work must never claim success"
    );
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Paused
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    let continuation = |store: &Store| ContinueAgent {
        request_id: RequestId::new(),
        expected_revision: store.snapshot().unwrap().streams[0].revision,
        previous_command_id: first.id,
        prompt: "Continue from the preserved work".into(),
    };
    assert!(store
        .continue_agent(first.stream_id, continuation(&store))
        .is_err());
    store
        .set_control(
            first.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Resume,
            },
        )
        .unwrap();
    assert!(
        store
            .continue_agent(first.stream_id, continuation(&store))
            .is_err(),
        "Final archive still required"
    );
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
    let follow = store
        .continue_agent(first.stream_id, continuation(&store))
        .unwrap();
    assert_eq!(follow.session_id(), first.session_id());
    assert_ne!(follow.id, first.id);
    assert_eq!(
        follow.agent_spec.unwrap().profile,
        first.agent_spec.unwrap().profile
    );
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
fn large_archive_blob_is_hidden_until_verified_and_replays_chunks_after_restart() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let c = command(&store);
    let bytes = vec![0x5a; 5 * 1024 * 1024 + 17];
    let file = ArchiveBlob {
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        bytes: bytes.len() as u64,
        uri: "file:///work/large.bin".into(),
        name: "../../large.bin".into(),
        mime: "application/octet-stream".into(),
        message_id: "msg_large".into(),
        source_revision: 1,
        captured_unix_ms: 100,
    };
    let first = ReportArchiveBlob {
        ownership_epoch: 1,
        chunk: ArchiveBlobChunk::new(file.clone(), 0, &bytes[..ARCHIVE_CHUNK_LIMIT]),
    };
    assert!(
        store
            .report_archive_blob(c.runner_id, c.id, first.clone())
            .is_err(),
        "unreferenced bytes must not be admitted"
    );
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
                    messages: vec![
                        json!({"id":"msg_large","content":[{"uri":file.uri,"mime":file.mime}]}),
                    ],
                },
            },
        )
        .unwrap();
    assert!(
        !store
            .report_archive_blob(c.runner_id, c.id, first.clone())
            .unwrap()
            .complete
    );
    assert!(store.archive_blobs(c.id, None).unwrap().files.is_empty());
    assert!(store.archive_blob_chunk(c.id, &file.sha256, 0).is_err());
    let mut altered = first.clone();
    altered.chunk.data = "AAAA".into();
    assert!(store
        .report_archive_blob(c.runner_id, c.id, altered)
        .is_err());
    let mut foreign = first.clone();
    foreign.ownership_epoch = 2;
    assert!(store
        .report_archive_blob(c.runner_id, c.id, foreign)
        .is_err());
    assert!(store
        .report_archive_blob(RunnerId::new(), c.id, first.clone())
        .is_err());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .report_archive_blob(c.runner_id, c.id, first.clone())
            .unwrap()
            .next_offset,
        ARCHIVE_CHUNK_LIMIT as u64
    );
    for offset in (ARCHIVE_CHUNK_LIMIT..bytes.len()).step_by(ARCHIVE_CHUNK_LIMIT) {
        let end = (offset + ARCHIVE_CHUNK_LIMIT).min(bytes.len());
        let mut report = ReportArchiveBlob {
            ownership_epoch: 1,
            chunk: ArchiveBlobChunk::new(file.clone(), offset as u64, &bytes[offset..end]),
        };
        if end == bytes.len() {
            let valid = report.clone();
            report.chunk =
                ArchiveBlobChunk::new(file.clone(), offset as u64, &vec![0; end - offset]);
            assert!(store
                .report_archive_blob(c.runner_id, c.id, report)
                .is_err());
            assert!(store.archive_blobs(c.id, None).unwrap().files.is_empty());
            report = valid;
        }
        assert_eq!(
            store
                .report_archive_blob(c.runner_id, c.id, report)
                .unwrap()
                .complete,
            end == bytes.len()
        );
    }
    assert_eq!(
        store.archive_blobs(c.id, Some(1)).unwrap().files,
        vec![file.clone()]
    );
    assert!(store.archive_blobs(c.id, Some(0)).unwrap().files.is_empty());
    let mut downloaded = Vec::new();
    while downloaded.len() < bytes.len() {
        let chunk = store
            .archive_blob_chunk(c.id, &file.sha256, downloaded.len() as u64)
            .unwrap();
        assert_eq!(chunk.file, file);
        downloaded.extend(chunk.bytes().unwrap());
    }
    assert_eq!(downloaded, bytes);
    assert!(
        store
            .report_archive_blob(c.runner_id, c.id, first)
            .unwrap()
            .complete
    );
    assert_eq!(
        store
            .events(0, 200)
            .unwrap()
            .events
            .iter()
            .filter(|e| e.kind == "archive.file")
            .count(),
        1
    );
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
