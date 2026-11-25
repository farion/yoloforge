use factory_core::*;
use factory_protocol::*;
use factoryd::store::{Store, StoreError};
use serde_json::json;
use std::sync::{Arc, Barrier};
#[path = "cases/termination.rs"]
mod termination_cases;

fn setup(store: &Store) -> (RunnerId, RunnerCommand) {
    let p = store
        .create_project(CreateProject {
            logo: None,
            runner_mapping: None,
            key: None,
            request_id: RequestId::new(),
            name: "questions".into(),
            repository: "/repo".into(),
            base_branch: "main".into(),
        })
        .unwrap();
    store
        .update_profile(
            p.id,
            UpdateProfile {
                request_id: RequestId::new(),
                expected_revision: Revision(1),
                profile: ExecutionProfile {
                    provider_id: "openai".into(),
                    model_id: "test".into(),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    let runner = RunnerId::new();
    let stream = store
        .create_stream(CreateStream {
            request_id: RequestId::new(),
            project_id: p.id,
            goal: "ask".into(),
            runner_id: runner,
            ticket_ids: vec![],
        })
        .unwrap();
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
                        path: "/workspace".into(),
                        branch: stream.branch,
                        base_commit: "a".repeat(40),
                    },
                },
            },
        )
        .unwrap();
    let cmd = store
        .enqueue(
            stream.id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                kind: CommandKind::RunAgent,
                prompt: Some("Ask me".into()),
            },
        )
        .unwrap();
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
    (runner, cmd)
}
fn external(command: CommandId) -> ExternalQuestion {
    ExternalQuestion {
        external_id: "form_factory_test".into(),
        kind: QuestionKind::Form,
        payload: json!({"id":"form_factory_test","sessionID":agent_session_id(command),"title":"Choose","fields":[{"key":"choice","type":"string","required":true}]}),
    }
}

#[test]
fn desktop_notifications_are_deduplicated_persistent_and_read_per_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notifications.sqlite");
    let store = Store::open(&path).unwrap();
    let (runner, cmd) = setup(&store);
    let body = QuestionSnapshot {
        ownership_epoch: 1,
        questions: vec![external(cmd.id)],
    };
    store.sync_questions(runner, cmd.id, body.clone()).unwrap();
    store.sync_questions(runner, cmd.id, body).unwrap();
    let page = store.notifications("desktop-a", None).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.unread, 1);
    assert_eq!(page.entries[0].command_id, Some(cmd.id));
    assert!(page.entries[0].project_id.is_some());
    let mark = MarkNotifications {
        request_id: RequestId::new(),
        ids: vec![page.entries[0].id],
        read: true,
    };
    store.mark_notifications("desktop-a", mark.clone()).unwrap();
    store.mark_notifications("desktop-a", mark).unwrap();
    assert_eq!(store.notifications("desktop-a", None).unwrap().unread, 0);
    assert_eq!(store.notifications("desktop-b", None).unwrap().unread, 1);
    assert!(store
        .notifications("desktop-a", Some(page.entries[0].id))
        .unwrap()
        .entries
        .is_empty());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(store.notifications("desktop-a", None).unwrap().entries[0].read);
    assert_eq!(store.notifications("desktop-b", None).unwrap().unread, 1);
}

#[test]
fn excerpts_survive_restart_and_reject_stale_foreign_and_oversized_samples() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("excerpt.sqlite");
    let store = Store::open(&path).unwrap();
    let (runner, cmd) = setup(&store);
    let body = ReportExcerpt {
        ownership_epoch: 1,
        excerpt: SessionExcerpt {
            session_title: Some("OpenCode: Farben abstimmen".into()),
            session_id: agent_session_id(cmd.id),
            revision: Revision(1),
            captured_unix_ms: 1,
            older_omitted: false,
            messages: vec![ExcerptMessage {
                id: "msg_test".into(),
                kind: "assistant".into(),
                text: "tool output".into(),
                truncated: false,
            }],
        },
    };
    assert!(store
        .report_excerpt(RunnerId::new(), cmd.id, body.clone())
        .is_err());
    assert_eq!(
        store.report_excerpt(runner, cmd.id, body.clone()).unwrap(),
        body.excerpt
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.excerpt(cmd.id).unwrap(), Some(body.excerpt.clone()));
    assert_eq!(
        store
            .snapshot()
            .unwrap()
            .session_titles
            .get(&agent_session_id(cmd.id))
            .map(String::as_str),
        Some("OpenCode: Farben abstimmen")
    );
    assert_eq!(
        store.report_excerpt(runner, cmd.id, body.clone()).unwrap(),
        body.excerpt
    );
    let mut changed = body.clone();
    changed.excerpt.messages[0].text = "changed".into();
    assert!(store
        .report_excerpt(runner, cmd.id, changed.clone())
        .is_err());
    changed.excerpt.revision = Revision(2);
    store
        .report_excerpt(runner, cmd.id, changed.clone())
        .unwrap();
    assert!(store.report_excerpt(runner, cmd.id, body).is_err());
    let mut oversized = changed.clone();
    oversized.excerpt.revision = Revision(3);
    oversized.excerpt.messages[0].text = "x".repeat(8193);
    assert!(store.report_excerpt(runner, cmd.id, oversized).is_err());
    let mut wrong = changed.clone();
    wrong.excerpt.revision = Revision(3);
    wrong.excerpt.session_id = agent_session_id(CommandId::new());
    assert!(store.report_excerpt(runner, cmd.id, wrong).is_err());
    assert_eq!(store.excerpt(cmd.id).unwrap(), Some(changed.excerpt));
}

#[test]
fn durable_pause_blocks_answers_and_requires_pre_execution_hold_for_resume() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("controls.sqlite");
    let store = Store::open(&path).unwrap();
    let (runner, cmd) = setup(&store);
    let q = sync(&store, runner, cmd.id, vec![external(cmd.id)]).remove(0);
    let body = SetControl {
        request_id: RequestId::new(),
        expected_revision: store.snapshot().unwrap().streams[0].revision,
        action: ControlAction::Pause,
    };
    let control = store.set_control(cmd.stream_id, body.clone()).unwrap();
    assert_eq!(control.state, PauseState::Requested);
    assert_eq!(store.set_control(cmd.stream_id, body).unwrap(), control);
    assert!(matches!(
        store.answer_question(
            q.id,
            AnswerQuestion {
                request_id: RequestId::new(),
                legacy_revision: None,
                answer: json!({"choice":"yes"})
            }
        ),
        Err(StoreError::WorkConflict)
    ));
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.runner_control(runner, cmd.id, 1).unwrap(),
        Some(control.clone())
    );
    assert!(store.runner_control(RunnerId::new(), cmd.id, 1).is_err());
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
    store
        .report_control(
            runner,
            cmd.id,
            ReportControl {
                proof: None,
                control_request: control.request_id,
                ownership_epoch: 1,
                state: PauseState::HeldBeforeExecution,
                summary: "No prompt submitted".into(),
            },
        )
        .unwrap();
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Paused
    );
    store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Resume,
            },
        )
        .unwrap();
    let next = store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Pause,
            },
        )
        .unwrap();
    assert!(store
        .report_control(
            runner,
            cmd.id,
            ReportControl {
                proof: None,
                control_request: control.request_id,
                ownership_epoch: 1,
                state: PauseState::HeldBeforeExecution,
                summary: "Stale report".into()
            }
        )
        .is_err());
    store
        .report_control(
            runner,
            cmd.id,
            ReportControl {
                proof: None,
                control_request: next.request_id,
                ownership_epoch: 1,
                state: PauseState::InterruptUnverified,
                summary: "Interrupt uncertain".into(),
            },
        )
        .unwrap();
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Blocked
    );
    assert!(store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Resume
            }
        )
        .is_err());
}
#[test]
fn freezer_control_requires_same_incarnation_and_confirmed_thaw_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("freezer.sqlite");
    let store = Store::open(&path).unwrap();
    let (runner, cmd) = setup(&store);
    let pause = store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Pause,
            },
        )
        .unwrap();
    let proof = ContainmentProof {
        container_id: "a".repeat(64),
        started_at: "2026-11-21T10:00:00Z".into(),
        frozen: true,
        observed_unix_ms: 1000,
    };
    let report = ReportControl {
        proof: Some(proof.clone()),
        control_request: pause.request_id,
        ownership_epoch: 1,
        state: PauseState::Frozen,
        summary: "Container freezer confirmed".into(),
    };
    let mut invalid = report.clone();
    invalid.proof = None;
    assert!(store.report_control(runner, cmd.id, invalid).is_err());
    let mut invalid = report.clone();
    invalid.ownership_epoch = 2;
    assert!(store.report_control(runner, cmd.id, invalid).is_err());
    assert!(store
        .report_control(RunnerId::new(), cmd.id, report.clone())
        .is_err());
    let frozen = store
        .report_control(runner, cmd.id, report.clone())
        .unwrap();
    assert_eq!(
        store
            .report_control(runner, cmd.id, report.clone())
            .unwrap(),
        frozen
    );
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Paused
    );
    let completion = CommandResult {
        request_id: RequestId::new(),
        ownership_epoch: 1,
        result: CommandOutput::Agent {
            agent: AgentProgress {
                session_id: cmd.session_id(),
                message_id: agent_message_id(cmd.id),
                phase: AgentPhase::Succeeded,
                summary: "done".into(),
            },
        },
    };
    assert!(matches!(
        store.complete(runner, cmd.id, completion.clone()),
        Err(StoreError::WorkConflict)
    ));
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.runner_control(runner, cmd.id, 1).unwrap(),
        Some(frozen)
    );
    let resume = SetControl {
        request_id: RequestId::new(),
        expected_revision: store.snapshot().unwrap().streams[0].revision,
        action: ControlAction::Resume,
    };
    let requested = store.set_control(cmd.stream_id, resume.clone()).unwrap();
    assert_eq!(requested.state, PauseState::ResumeRequested);
    assert_eq!(store.set_control(cmd.stream_id, resume).unwrap(), requested);
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Pausing
    );
    assert!(store.report_control(runner, cmd.id, report).is_err());
    assert!(matches!(
        store.complete(runner, cmd.id, completion),
        Err(StoreError::WorkConflict)
    ));
    let thaw = ReportControl {
        proof: Some(ContainmentProof {
            frozen: false,
            observed_unix_ms: 1001,
            ..proof
        }),
        control_request: requested.request_id,
        ownership_epoch: 1,
        state: PauseState::Released,
        summary: "Same runtime thawed".into(),
    };
    let mut wrong = thaw.clone();
    wrong.proof.as_mut().unwrap().container_id = "b".repeat(64);
    assert!(store.report_control(runner, cmd.id, wrong).is_err());
    let mut wrong = thaw.clone();
    wrong.proof.as_mut().unwrap().started_at = "2026-11-21T11:00:00Z".into();
    assert!(store.report_control(runner, cmd.id, wrong).is_err());
    let mut wrong = thaw.clone();
    wrong.proof.as_mut().unwrap().observed_unix_ms = 999;
    assert!(store.report_control(runner, cmd.id, wrong).is_err());
    let mut wrong = thaw.clone();
    wrong.proof.as_mut().unwrap().frozen = true;
    assert!(store.report_control(runner, cmd.id, wrong).is_err());
    let released = store.report_control(runner, cmd.id, thaw.clone()).unwrap();
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Running
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.report_control(runner, cmd.id, thaw).unwrap(),
        released
    );
}

#[test]
fn uncertain_thaw_preserves_identity_and_cannot_be_reclassified_as_pre_execution_hold() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let (runner, cmd) = setup(&store);
    let pause = store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Pause,
            },
        )
        .unwrap();
    let proof = ContainmentProof {
        container_id: "a".repeat(64),
        started_at: "2026-11-21T10:00:00Z".into(),
        frozen: true,
        observed_unix_ms: 1000,
    };
    store
        .report_control(
            runner,
            cmd.id,
            ReportControl {
                proof: Some(proof.clone()),
                control_request: pause.request_id,
                ownership_epoch: 1,
                state: PauseState::Frozen,
                summary: "frozen".into(),
            },
        )
        .unwrap();
    let resume = store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Resume,
            },
        )
        .unwrap();
    let uncertain = ReportControl {
        proof: None,
        control_request: resume.request_id,
        ownership_epoch: 1,
        state: PauseState::InterruptUnverified,
        summary: "Thaw outcome unknown".into(),
    };
    let saved = store
        .report_control(runner, cmd.id, uncertain.clone())
        .unwrap();
    assert_eq!(saved.proof, Some(proof.clone()));
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Blocked
    );
    let mut invalid = uncertain;
    invalid.state = PauseState::HeldBeforeExecution;
    assert!(store.report_control(runner, cmd.id, invalid).is_err());
    assert!(store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Resume
            }
        )
        .is_err());
    // Read-only reconciliation can confirm the original thaw without a new operation.
    store
        .report_control(
            runner,
            cmd.id,
            ReportControl {
                proof: Some(ContainmentProof {
                    frozen: false,
                    observed_unix_ms: 1002,
                    ..proof
                }),
                control_request: resume.request_id,
                ownership_epoch: 1,
                state: PauseState::Released,
                summary: "Thaw reconciled".into(),
            },
        )
        .unwrap();
}

#[test]
fn fresh_freezer_observations_do_not_invalidate_a_pending_operator_resume() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let (runner, cmd) = setup(&store);
    let pause = store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Pause,
            },
        )
        .unwrap();
    let mut body = ReportControl {
        proof: Some(ContainmentProof {
            container_id: "a".repeat(64),
            started_at: "2026-11-21T10:00:00Z".into(),
            frozen: true,
            observed_unix_ms: 1000,
        }),
        control_request: pause.request_id,
        ownership_epoch: 1,
        state: PauseState::Frozen,
        summary: "frozen".into(),
    };
    store.report_control(runner, cmd.id, body.clone()).unwrap();
    let operator_revision = store.snapshot().unwrap().streams[0].revision;
    body.proof.as_mut().unwrap().observed_unix_ms = 1001;
    store.report_control(runner, cmd.id, body.clone()).unwrap();
    assert_eq!(
        store.snapshot().unwrap().streams[0].revision,
        operator_revision
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .runner_control(runner, cmd.id, 1)
            .unwrap()
            .unwrap()
            .proof,
        body.proof
    );
    let resumed = store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: operator_revision,
                action: ControlAction::Resume,
            },
        )
        .unwrap();
    assert_eq!(resumed.state, PauseState::ResumeRequested);
    assert!(store.snapshot().unwrap().streams[0].revision.0 > operator_revision.0);
}

fn uncertain_thaw(store: &Store) -> (RunnerId, RunnerCommand, StreamControl) {
    let (runner, cmd) = setup(store);
    let pause = store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Pause,
            },
        )
        .unwrap();
    store
        .report_control(
            runner,
            cmd.id,
            ReportControl {
                control_request: pause.request_id,
                ownership_epoch: 1,
                state: PauseState::Frozen,
                summary: "frozen".into(),
                proof: Some(ContainmentProof {
                    container_id: "a".repeat(64),
                    started_at: "2026-11-21T10:00:00Z".into(),
                    frozen: true,
                    observed_unix_ms: 1000,
                }),
            },
        )
        .unwrap();
    let resume = store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Resume,
            },
        )
        .unwrap();
    let unknown = store
        .report_control(
            runner,
            cmd.id,
            ReportControl {
                control_request: resume.request_id,
                ownership_epoch: 1,
                state: PauseState::InterruptUnverified,
                summary: "thaw uncertain".into(),
                proof: None,
            },
        )
        .unwrap();
    (runner, cmd, unknown)
}
fn retry_body(store: &Store, unknown: &StreamControl) -> RetryControl {
    RetryControl {
        request_id: RequestId::new(),
        expected_revision: store.snapshot().unwrap().streams[0].revision,
        previous_request: unknown.request_id,
        expected_proof: unknown.proof.clone().unwrap(),
        reason: "Operator checked the dedicated runtime; retry the original thaw".into(),
    }
}

#[test]
fn concurrent_control_recovery_has_one_winner_and_replays_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let (runner, cmd, unknown) = uncertain_thaw(&store);
    let body = retry_body(&store, &unknown);
    let barrier = Arc::new(Barrier::new(2));
    let workers = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            let mut body = body.clone();
            body.request_id = RequestId::new();
            std::thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                barrier.wait();
                let result = store.retry_control(cmd.stream_id, body.clone());
                (body, result)
            })
        })
        .collect::<Vec<_>>();
    let outcomes = workers
        .into_iter()
        .map(|w| w.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|(_, r)| r.is_ok()).count(), 1);
    assert!(outcomes
        .iter()
        .any(|(_, r)| matches!(r, Err(StoreError::RevisionConflict(_)))));
    let (body, result) = outcomes.into_iter().find(|(_, r)| r.is_ok()).unwrap();
    let requested = result.unwrap();
    assert_eq!(requested.state, PauseState::ResumeRequested);
    assert_eq!(requested.action, unknown.action);
    assert_eq!(requested.proof, unknown.proof);
    assert_eq!(
        requested.retry.as_ref().unwrap().previous_request,
        unknown.request_id
    );
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
    assert_eq!(
        store.snapshot().unwrap().streams[0].status,
        StreamState::Pausing
    );
    assert!(store
        .report_control(
            runner,
            cmd.id,
            ReportControl {
                control_request: unknown.request_id,
                ownership_epoch: 1,
                state: PauseState::Released,
                summary: "late thaw".into(),
                proof: Some(ContainmentProof {
                    frozen: false,
                    observed_unix_ms: 1001,
                    ..body.expected_proof.clone()
                })
            }
        )
        .is_err());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.retry_control(cmd.stream_id, body.clone()).unwrap(),
        requested
    );
    store
        .report_control(
            runner,
            cmd.id,
            ReportControl {
                control_request: requested.request_id,
                ownership_epoch: 1,
                state: PauseState::Released,
                summary: "new thaw confirmed".into(),
                proof: Some(ContainmentProof {
                    frozen: false,
                    observed_unix_ms: 1002,
                    ..body.expected_proof.clone()
                }),
            },
        )
        .unwrap();
    assert_eq!(
        store.retry_control(cmd.stream_id, body.clone()).unwrap(),
        requested
    );
    let mut changed = body;
    changed.reason = "Changed justification".into();
    assert!(store.retry_control(cmd.stream_id, changed).is_err());
    assert_eq!(
        store
            .events(0, 200)
            .unwrap()
            .events
            .iter()
            .filter(|e| e.kind == "stream.control.retry")
            .count(),
        1
    );
}

#[test]
fn control_recovery_rejects_changed_evidence_and_rolls_back_on_event_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let (runner, cmd, unknown) = uncertain_thaw(&store);
    let body = retry_body(&store, &unknown);
    let mut wrong = body.clone();
    wrong.expected_proof.container_id = "b".repeat(64);
    assert!(store.retry_control(cmd.stream_id, wrong).is_err());
    let mut wrong = body.clone();
    wrong.previous_request = RequestId::new();
    assert!(store.retry_control(cmd.stream_id, wrong).is_err());
    let mut wrong = body.clone();
    wrong.reason = "\n".into();
    assert!(store.retry_control(cmd.stream_id, wrong).is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_retry BEFORE INSERT ON events WHEN NEW.kind='stream.control.retry' BEGIN SELECT RAISE(ABORT,'test rollback'); END;").unwrap();
    assert!(store.retry_control(cmd.stream_id, body.clone()).is_err());
    assert_eq!(
        store.runner_control(runner, cmd.id, 1).unwrap(),
        Some(unknown)
    );
    assert_eq!(
        store.snapshot().unwrap().streams[0].revision,
        body.expected_revision
    );
    db.execute_batch("DROP TRIGGER fail_retry").unwrap();
    store.retry_control(cmd.stream_id, body).unwrap();
}

#[test]
fn native_interrupt_without_container_evidence_cannot_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let (runner, cmd) = setup(&store);
    let pause = store
        .set_control(
            cmd.stream_id,
            SetControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                action: ControlAction::Pause,
            },
        )
        .unwrap();
    store
        .report_control(
            runner,
            cmd.id,
            ReportControl {
                control_request: pause.request_id,
                ownership_epoch: 1,
                state: PauseState::InterruptUnverified,
                summary: "native interrupt uncertain".into(),
                proof: None,
            },
        )
        .unwrap();
    assert!(store
        .retry_control(
            cmd.stream_id,
            RetryControl {
                request_id: RequestId::new(),
                expected_revision: store.snapshot().unwrap().streams[0].revision,
                previous_request: pause.request_id,
                expected_proof: ContainmentProof {
                    container_id: "a".repeat(64),
                    started_at: "fake".into(),
                    frozen: true,
                    observed_unix_ms: 1
                },
                reason: "operator supplied evidence cannot replace runner proof".into()
            }
        )
        .is_err());
}

#[tokio::test]
async fn control_recovery_http_round_trip_is_authenticated_and_deduplicated() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let (_, cmd, unknown) = uncertain_thaw(&store);
    let body = retry_body(&store, &unknown);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/v1/streams/{}/control/retry",
        listener.local_addr().unwrap(),
        cmd.stream_id
    );
    let token = "test-only-control-token-0123456789abcdef";
    let task = tokio::spawn(async move {
        axum::serve(listener, factoryd::router(token, store))
            .await
            .unwrap()
    });
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .post(&url)
            .header(VERSION_HEADER, "1")
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let response = client
        .post(&url)
        .bearer_auth(token)
        .header(VERSION_HEADER, "1")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let saved = response.json::<StreamControl>().await.unwrap();
    let replay = client
        .post(&url)
        .bearer_auth(token)
        .header(VERSION_HEADER, "1")
        .json(&body)
        .send()
        .await
        .unwrap()
        .json::<StreamControl>()
        .await
        .unwrap();
    assert_eq!(replay, saved);
    assert_eq!(saved.retry.unwrap().reason, body.reason);
    task.abort();
}

fn sync(
    store: &Store,
    runner: RunnerId,
    cmd: CommandId,
    questions: Vec<ExternalQuestion>,
) -> Vec<Question> {
    store
        .sync_questions(
            runner,
            cmd,
            QuestionSnapshot {
                ownership_epoch: 1,
                questions,
            },
        )
        .unwrap()
}

#[test]
fn first_answer_wins_without_revision_check_and_replay_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let a = Store::open(&path).unwrap();
    let b = Store::open(&path).unwrap();
    let (runner, cmd) = setup(&a);
    let question = sync(&a, runner, cmd.id, vec![external(cmd.id)])[0].clone();
    let input = AnswerQuestion {
        request_id: RequestId::new(),
        legacy_revision: Some(Revision(0)),
        answer: json!({"choice":"A"}),
    };
    let retry = input.clone();
    let gate = Arc::new(Barrier::new(2));
    let other_gate = gate.clone();
    let cloned = a.clone();
    let task = std::thread::spawn(move || {
        other_gate.wait();
        cloned.answer_question(question.id, input)
    });
    gate.wait();
    let second = b.answer_question(
        question.id,
        AnswerQuestion {
            request_id: RequestId::new(),
            legacy_revision: Some(Revision(0)),
            answer: json!({"choice":"B"}),
        },
    );
    let first = task.join().unwrap();
    assert!(matches!(
        (&first, &second),
        (Ok(_), Err(StoreError::WorkConflict))
            | (Err(StoreError::WorkConflict), Ok(_))
    ));
    if let Ok(first) = first {
        assert_eq!(a.answer_question(question.id, retry).unwrap(), first);
    }
    let queued = a.snapshot().unwrap().questions[0].clone();
    let cursor = a.snapshot().unwrap().cursor;
    sync(&a, runner, cmd.id, vec![external(cmd.id)]);
    assert_eq!(a.snapshot().unwrap().cursor, cursor);
    drop(a);
    drop(b);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot().unwrap().questions, vec![queued.clone()]);
    let delivery = AnswerDelivery {
        request_id: RequestId::new(),
        ownership_epoch: 1,
        answer_request: queued.answer_request.unwrap(),
        state: QuestionState::Answered,
    };
    assert!(matches!(
        store.answer_delivery(RunnerId::new(), queued.id, delivery.clone()),
        Err(StoreError::WorkConflict)
    ));
    let answered = store
        .answer_delivery(runner, queued.id, delivery.clone())
        .unwrap();
    let cursor = store.snapshot().unwrap().cursor;
    assert_eq!(
        store.answer_delivery(runner, queued.id, delivery).unwrap(),
        answered
    );
    assert_eq!(store.snapshot().unwrap().cursor, cursor);
}

#[test]
fn missing_or_changed_questions_remain_unknown_and_are_not_recreated() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let (runner, cmd) = setup(&store);
    let original = external(cmd.id);
    let question = sync(&store, runner, cmd.id, vec![original.clone()])[0].clone();
    let missing = sync(&store, runner, cmd.id, vec![])[0].clone();
    assert_eq!(missing.state, QuestionState::Unknown);
    let reappeared = sync(&store, runner, cmd.id, vec![original])[0].clone();
    assert_eq!(reappeared, missing);
    assert!(matches!(
        store.answer_question(
            question.id,
            AnswerQuestion {
                request_id: RequestId::new(),
                legacy_revision: None,
                answer: json!({"choice":"unsafe"})
            }
        ),
        Err(StoreError::WorkConflict)
    ));
    let mut new = external(cmd.id);
    new.external_id = "form_new".into();
    new.payload["id"] = json!("form_new");
    let created = sync(&store, runner, cmd.id, vec![new.clone()]);
    let fresh = created.last().unwrap().clone();
    new.payload["fields"] = json!([]);
    let changed = sync(&store, runner, cmd.id, vec![new]);
    assert_eq!(changed.last().unwrap().id, fresh.id);
    assert_eq!(changed.last().unwrap().state, QuestionState::Unknown);
    assert_eq!(changed.last().unwrap().external, fresh.external);
}

#[test]
fn queued_answer_missing_after_reply_can_be_confirmed_and_rejected_answer_can_be_corrected() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let (runner, cmd) = setup(&store);
    let question = sync(&store, runner, cmd.id, vec![external(cmd.id)])[0].clone();
    let queued = store
        .answer_question(
            question.id,
            AnswerQuestion {
                request_id: RequestId::new(),
                legacy_revision: None,
                answer: json!({"choice":"A"}),
            },
        )
        .unwrap();
    let rejected = store
        .answer_delivery(
            runner,
            question.id,
            AnswerDelivery {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                answer_request: queued.answer_request.unwrap(),
                state: QuestionState::Rejected,
            },
        )
        .unwrap();
    assert_eq!(rejected.state, QuestionState::Rejected);
    let corrected = store
        .answer_question(
            question.id,
            AnswerQuestion {
                request_id: RequestId::new(),
                legacy_revision: None,
                answer: json!({"choice":"B"}),
            },
        )
        .unwrap();
    sync(&store, runner, cmd.id, vec![]);
    let answered = store
        .answer_delivery(
            runner,
            question.id,
            AnswerDelivery {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                answer_request: corrected.answer_request.unwrap(),
                state: QuestionState::Answered,
            },
        )
        .unwrap();
    assert_eq!(answered.state, QuestionState::Answered);
    assert_eq!(answered.answer, Some(json!({"choice":"B"})));
}
