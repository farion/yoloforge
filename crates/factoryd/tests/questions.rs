use factory_core::*;
use factory_protocol::*;
use factoryd::store::{Store, StoreError};
use serde_json::json;
use std::sync::{Arc, Barrier};

fn setup(store: &Store) -> (RunnerId, RunnerCommand) {
    let p = store
        .create_project(CreateProject {
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
fn excerpts_survive_restart_and_reject_stale_foreign_and_oversized_samples() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("excerpt.sqlite");
    let store = Store::open(&path).unwrap();
    let (runner, cmd) = setup(&store);
    let body = ReportExcerpt {
        ownership_epoch: 1,
        excerpt: SessionExcerpt {
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
                expected_revision: q.revision,
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
fn concurrent_answers_are_revision_protected_and_replay_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let a = Store::open(&path).unwrap();
    let b = Store::open(&path).unwrap();
    let (runner, cmd) = setup(&a);
    let question = sync(&a, runner, cmd.id, vec![external(cmd.id)])[0].clone();
    let input = AnswerQuestion {
        request_id: RequestId::new(),
        expected_revision: Revision(1),
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
            expected_revision: Revision(1),
            answer: json!({"choice":"B"}),
        },
    );
    let first = task.join().unwrap();
    assert!(matches!(
        (&first, &second),
        (Ok(_), Err(StoreError::RevisionConflict(_)))
            | (Err(StoreError::RevisionConflict(_)), Ok(_))
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
                expected_revision: missing.revision,
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
                expected_revision: question.revision,
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
    let corrected = store
        .answer_question(
            question.id,
            AnswerQuestion {
                request_id: RequestId::new(),
                expected_revision: rejected.revision,
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
