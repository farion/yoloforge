use super::*;
#[test]
fn checkpoint_requires_stopped_writer_and_atomically_pins_context() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let (runner, cmd, c) = uncertain_thaw(&store);
    let enqueue = |store: &Store| EnqueueCommand {
        request_id: RequestId::new(),
        expected_revision: store.snapshot().unwrap().streams[0].revision,
        kind: CommandKind::Checkpoint,
        prompt: None,
    };
    assert!(store.enqueue(cmd.stream_id, enqueue(&store)).is_err());
    let t = store
        .request_termination(cmd.id, request(&store, &c))
        .unwrap();
    assert!(store.enqueue(cmd.stream_id, enqueue(&store)).is_err());
    store
        .report_termination(runner, cmd.id, stopped(&t))
        .unwrap();
    let body = enqueue(&store);
    let checkpoint = store.enqueue(cmd.stream_id, body.clone()).unwrap();
    assert_eq!(store.enqueue(cmd.stream_id, body).unwrap(), checkpoint);
    let spec = store.checkpoint(checkpoint.id).unwrap().unwrap();
    assert_eq!(spec.source_commands, vec![cmd.id]);
    assert_eq!(spec.stopped_runtimes, vec![stopped(&t).proof.unwrap()]);
    assert!(store.enqueue(cmd.stream_id, enqueue(&store)).is_err());
    store
        .acknowledge(
            runner,
            checkpoint.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
    assert_eq!(
        store.runner_checkpoint(runner, checkpoint.id, 1).unwrap(),
        spec
    );
    assert!(store
        .runner_checkpoint(RunnerId::new(), checkpoint.id, 1)
        .is_err());
    let receipt = store
        .upload_checkpoint_bundle(runner, checkpoint.id, 1, b"bundle fixture".to_vec())
        .unwrap();
    assert_eq!(
        store
            .upload_checkpoint_bundle(runner, checkpoint.id, 1, b"bundle fixture".to_vec())
            .unwrap(),
        receipt
    );
    assert!(store
        .upload_checkpoint_bundle(runner, checkpoint.id, 1, b"changed".to_vec())
        .is_err());
    let result = GitCheckpoint {
        commit: "a".repeat(40),
        parent: "b".repeat(40),
        tree: "c".repeat(40),
        bundle_path: "/checkpoints/a.bundle".into(),
        bundle_sha256: receipt.sha256,
        bundle_bytes: receipt.bytes,
        manifest_path: "/checkpoints/manifest.json".into(),
    };
    store
        .complete(
            runner,
            checkpoint.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Checkpoint { checkpoint: result },
            },
        )
        .unwrap();
    assert_eq!(store.checkpoint(checkpoint.id).unwrap().unwrap(), spec);
    let next = RunnerId::new();
    let body = HandoffStream {
        request_id: RequestId::new(),
        expected_revision: store.snapshot().unwrap().streams[0].revision,
        checkpoint_command: checkpoint.id,
        target_runner: next,
        reason: "Continue on laptop".into(),
    };
    let db = rusqlite::Connection::open(dir.path().join("db")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_handoff BEFORE INSERT ON events WHEN NEW.kind='stream.handoff' BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
    assert!(store.handoff(cmd.stream_id, body.clone()).is_err());
    assert_eq!(store.snapshot().unwrap().streams[0].runner_id, runner);
    db.execute_batch("DROP TRIGGER fail_handoff").unwrap();
    let moved = store.handoff(cmd.stream_id, body.clone()).unwrap();
    assert_eq!(moved.ownership_epoch, 2);
    assert_eq!(moved.runner_id, next);
    assert!(moved.branch.ends_with("epoch-2"));
    assert_eq!(store.handoff(cmd.stream_id, body.clone()).unwrap(), moved);
    assert!(store
        .upload_checkpoint_bundle(runner, checkpoint.id, 1, b"bundle fixture".to_vec())
        .is_ok());
    assert!(store
        .upload_checkpoint_bundle(runner, checkpoint.id, 1, b"changed".to_vec())
        .is_err());
    let mut competing = body;
    competing.request_id = RequestId::new();
    competing.target_runner = RunnerId::new();
    assert!(store.handoff(cmd.stream_id, competing).is_err());
    assert!(store.runner_checkpoint(runner, checkpoint.id, 1).is_err());
    assert!(store
        .enqueue(
            cmd.stream_id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: moved.revision,
                kind: CommandKind::RunAgent,
                prompt: Some("Premature".into())
            }
        )
        .is_err());
    let prepare = store
        .enqueue(
            cmd.stream_id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: moved.revision,
                kind: CommandKind::PrepareWorkspace,
                prompt: None,
            },
        )
        .unwrap();
    assert_eq!(prepare.ownership_epoch, 2);
    assert!(store
        .require_checkpoint_support(next, prepare.id, 2, false, false)
        .is_err());
    assert!(store
        .require_checkpoint_support(next, prepare.id, 2, true, false)
        .is_ok());
    assert_eq!(
        prepare.workspace_spec.as_ref().unwrap().checkpoint,
        moved.checkpoint
    );
    assert!(store
        .acknowledge(
            runner,
            prepare.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1
            }
        )
        .is_err());
    store
        .acknowledge(
            next,
            prepare.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 2,
            },
        )
        .unwrap();
    assert_eq!(
        store
            .download_checkpoint_bundle(next, prepare.id, 2)
            .unwrap(),
        b"bundle fixture"
    );
    assert!(store
        .download_checkpoint_bundle(runner, prepare.id, 1)
        .is_err());
}
#[test]
fn central_binding_is_immutable_and_authorizes_removal_without_freezer_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let (runner, cmd) = setup(&store);
    let bound = RuntimeBinding {
        command_id: cmd.id,
        stream_id: cmd.stream_id,
        runner_id: runner,
        ownership_epoch: 1,
        runtime: RuntimeRef {
            container_id: "a".repeat(64),
            started_at: "2026-11-21T00:00:00Z".into(),
        },
        daemon_id: "engine-original".into(),
        endpoint: "http://127.0.0.1:4096".into(),
        workspace: "/work/stream".into(),
    };
    assert!(store
        .report_runtime_binding(RunnerId::new(), cmd.id, bound.clone())
        .is_err());
    assert_eq!(
        store
            .report_runtime_binding(runner, cmd.id, bound.clone())
            .unwrap(),
        bound
    );
    let mut changed = bound.clone();
    changed.daemon_id = "foreign".into();
    assert!(store
        .report_runtime_binding(runner, cmd.id, changed)
        .is_err());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .report_runtime_binding(runner, cmd.id, bound.clone())
            .unwrap(),
        bound
    );
    assert!(store.snapshot().unwrap().controls.is_empty());
    let t = store
        .request_termination(
            cmd.id,
            RequestTermination {
                request_id: RequestId::new(),
                expected_stream_revision: store.snapshot().unwrap().streams[0].revision,
                runtime: bound.runtime.clone(),
                previous_request: None,
                reason: "Recover lost runner journal".into(),
            },
        )
        .unwrap();
    assert_eq!(t.binding, Some(bound));
    let mut wrong = stopped(&t);
    wrong.proof.as_mut().unwrap().daemon_id = "foreign".into();
    assert!(store.report_termination(runner, cmd.id, wrong).is_err());
    assert_eq!(
        store
            .report_termination(runner, cmd.id, stopped(&t))
            .unwrap()
            .state,
        TerminationState::Stopped
    );
}
#[tokio::test]
async fn termination_http_client_roundtrip_requires_current_runner_registration() {
    use factory_protocol::client::FactoryClient;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let (runner, cmd, c) = uncertain_thaw(&store);
    let body = request(&store, &c);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let token = "test-only-termination-token-0123456789abcdef";
    let task = tokio::spawn(async move {
        axum::serve(listener, factoryd::router(token, store))
            .await
            .unwrap()
    });
    let client = FactoryClient::new(&url, token).unwrap();
    let bad = FactoryClient::new(&url, "invalid-test-token-0123456789abcdef").unwrap();
    assert!(bad.request_termination(cmd.id, &body).await.is_err());
    let t = client.request_termination(cmd.id, &body).await.unwrap();
    assert_eq!(client.request_termination(cmd.id, &body).await.unwrap(), t);
    assert_eq!(client.termination(cmd.id).await.unwrap(), Some(t.clone()));
    assert!(client
        .report_termination(runner, RegistrationId::new(), cmd.id, &stopped(&t))
        .await
        .is_err());
    let registration = client
        .register(&RegisterRunner {
            capabilities: vec![RUNTIME_TERMINATION_CAPABILITY.into()],
            id: runner,
            instance_id: InstanceId::new(),
            name: "termination-test".into(),
            location: RunnerLocation::Local,
            version: "0.1.2-dev".into(),
            opencode: OpenCodeStatus::NotConfigured,
        })
        .await
        .unwrap();
    assert_eq!(
        client
            .runner_termination(runner, registration.registration_id, cmd.id, 1)
            .await
            .unwrap(),
        Some(t.clone())
    );
    let ended = client
        .report_termination(runner, registration.registration_id, cmd.id, &stopped(&t))
        .await
        .unwrap();
    assert_eq!(ended.state, TerminationState::Stopped);
    assert_eq!(
        client
            .report_termination(runner, registration.registration_id, cmd.id, &stopped(&t))
            .await
            .unwrap(),
        ended
    );
    task.abort();
}
fn request(store: &Store, control: &StreamControl) -> RequestTermination {
    let p = control.proof.as_ref().unwrap();
    RequestTermination {
        request_id: RequestId::new(),
        expected_stream_revision: store.snapshot().unwrap().streams[0].revision,
        runtime: RuntimeRef {
            container_id: p.container_id.clone(),
            started_at: p.started_at.clone(),
        },
        previous_request: None,
        reason: "Obsolete contained execution; preserve worktree and archived evidence".into(),
    }
}
fn stopped(t: &RuntimeTermination) -> ReportTermination {
    ReportTermination {
        request_id: t.request_id,
        ownership_epoch: t.ownership_epoch,
        state: TerminationState::Stopped,
        proof: Some(RemovalProof {
            runtime: t.runtime.clone(),
            daemon_id: "engine-original".into(),
            observed_unix_ms: 2000,
        }),
    }
}
#[test]
fn removal_is_atomic_fenced_and_retains_archive_but_never_claims_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let (runner, cmd, c) = uncertain_thaw(&store);
    store.report_archive(runner,cmd.id,ReportArchive {ownership_epoch:1,batch:ArchiveBatch {session_id:cmd.session_id(),revision:1,captured_unix_ms:1000,final_scan_complete:false,messages:vec![json!({"id":"message_preserved","text":"Important decision before termination"})]}}).unwrap();
    let q = sync(&store, runner, cmd.id, vec![external(cmd.id)])
        .pop()
        .unwrap();
    let body = request(&store, &c);
    let t = store.request_termination(cmd.id, body.clone()).unwrap();
    assert_eq!(store.request_termination(cmd.id, body).unwrap(), t);
    let revision = store.snapshot().unwrap().streams[0].revision;
    assert!(store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: revision,
                action: ControlAction::Resume
            }
        )
        .is_err());
    assert!(store
        .retry_control(cmd.stream_id, retry_body(&store, &c))
        .is_err());
    assert!(store
        .complete(
            runner,
            cmd.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Agent {
                    agent: AgentProgress {
                        session_id: cmd.session_id(),
                        message_id: agent_message_id(cmd.id),
                        phase: AgentPhase::Succeeded,
                        summary: "late success".into()
                    }
                }
            }
        )
        .is_err());
    let report = stopped(&t);
    assert!(store
        .report_termination(RunnerId::new(), cmd.id, report.clone())
        .is_err());
    let mut wrong = report.clone();
    wrong.ownership_epoch = 2;
    assert!(store.report_termination(runner, cmd.id, wrong).is_err());
    let mut wrong = report.clone();
    wrong.proof.as_mut().unwrap().runtime.started_at = "new incarnation".into();
    assert!(store.report_termination(runner, cmd.id, wrong).is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_termination BEFORE INSERT ON events WHEN NEW.kind='runtime.termination' AND json_extract(NEW.payload,'$.state')='stopped' BEGIN SELECT RAISE(ABORT,'test rollback'); END;").unwrap();
    assert!(store
        .report_termination(runner, cmd.id, report.clone())
        .is_err());
    assert_eq!(store.termination(cmd.id).unwrap(), Some(t.clone()));
    assert_eq!(
        store
            .snapshot()
            .unwrap()
            .commands
            .iter()
            .find(|v| v.id == cmd.id)
            .unwrap()
            .state,
        CommandState::Accepted
    );
    db.execute_batch("DROP TRIGGER fail_termination").unwrap();
    let confirmed = store
        .report_termination(runner, cmd.id, report.clone())
        .unwrap();
    assert!(confirmed.archive_may_be_incomplete);
    let snapshot = store.snapshot().unwrap();
    assert_eq!(
        snapshot
            .commands
            .iter()
            .find(|v| v.id == cmd.id)
            .unwrap()
            .state,
        CommandState::Failed
    );
    assert_eq!(
        snapshot
            .questions
            .iter()
            .find(|v| v.id == q.id)
            .unwrap()
            .state,
        QuestionState::Unknown
    );
    assert_eq!(snapshot.controls[0].state, PauseState::Released);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.report_termination(runner, cmd.id, report).unwrap(),
        confirmed
    );
    let archive = store.archive(cmd.id, 0, None).unwrap();
    assert_eq!(archive.entries.len(), 1);
    assert_eq!(archive.termination, Some(confirmed));
    assert!(!archive.status.unwrap().final_scan_complete);
    assert!(store
        .enqueue(
            cmd.stream_id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                kind: CommandKind::RunAgent,
                prompt: Some(
                    "Start a fresh session only after configuring a new dedicated runtime".into()
                )
            }
        )
        .is_ok());
}
#[test]
fn unknown_removal_can_only_be_superseded_explicitly_with_same_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let (runner, cmd, c) = uncertain_thaw(&store);
    let body = request(&store, &c);
    let old = store.request_termination(cmd.id, body.clone()).unwrap();
    let mut retry = body;
    retry.request_id = RequestId::new();
    retry.expected_stream_revision = store.snapshot().unwrap().streams[0].revision;
    retry.previous_request = Some(old.request_id);
    assert!(store.request_termination(cmd.id, retry.clone()).is_err());
    store
        .report_termination(
            runner,
            cmd.id,
            ReportTermination {
                request_id: old.request_id,
                ownership_epoch: 1,
                state: TerminationState::Unknown,
                proof: None,
            },
        )
        .unwrap();
    let mut wrong = retry.clone();
    wrong.runtime.container_id = "b".repeat(64);
    assert!(store.request_termination(cmd.id, wrong).is_err());
    let next = store.request_termination(cmd.id, retry).unwrap();
    assert_eq!(next.previous_request, Some(old.request_id));
    assert!(store
        .report_termination(runner, cmd.id, stopped(&old))
        .is_err());
    assert_eq!(
        store
            .report_termination(runner, cmd.id, stopped(&next))
            .unwrap()
            .state,
        TerminationState::Stopped
    );
}
#[test]
fn no_container_evidence_means_no_removal_and_no_forced_release() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let (_, cmd) = setup(&store);
    assert!(store
        .request_termination(
            cmd.id,
            RequestTermination {
                request_id: RequestId::new(),
                expected_stream_revision: store.snapshot().unwrap().streams[0].revision,
                runtime: RuntimeRef {
                    container_id: "a".repeat(64),
                    started_at: "invented".into()
                },
                previous_request: None,
                reason: "No proof".into()
            }
        )
        .is_err());
    assert_eq!(
        store
            .snapshot()
            .unwrap()
            .commands
            .iter()
            .find(|c| c.id == cmd.id)
            .unwrap()
            .state,
        CommandState::Accepted
    );
}
