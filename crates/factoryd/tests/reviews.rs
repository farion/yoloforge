use factory_core::*;
use factory_protocol::*;
use factoryd::store::Store;
fn rev(s: &Store, id: StreamId) -> Revision {
    s.snapshot()
        .unwrap()
        .streams
        .iter()
        .find(|v| v.id == id)
        .unwrap()
        .revision
}
fn ack(s: &Store, c: &RunnerCommand) {
    s.acknowledge(
        c.runner_id,
        c.id,
        AckCommand {
            request_id: RequestId::new(),
            ownership_epoch: c.ownership_epoch,
        },
    )
    .unwrap();
}
fn result(s: &Store, c: &RunnerCommand, r: CommandOutput) {
    s.complete(
        c.runner_id,
        c.id,
        CommandResult {
            request_id: RequestId::new(),
            ownership_epoch: c.ownership_epoch,
            result: r,
        },
    )
    .unwrap();
}
#[test]
fn review_pins_profile_and_source_and_only_matching_test_evidence_can_authorize_integration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let p = store
        .create_project(CreateProject {
            logo: None,
            runner_mapping: None,
            key: None,
            request_id: RequestId::new(),
            name: "Review".into(),
            repository: "https://example.invalid/repo.git".into(),
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
                    model_id: "test".into(),
                    test_command: "python3 -m unittest".into(),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    let s = store
        .create_stream(CreateStream {
            request_id: RequestId::new(),
            project_id: p.id,
            runner_id: RunnerId::new(),
            goal: "test".into(),
            ticket_ids: vec![],
        })
        .unwrap();
    let prepare = store
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
    ack(&store, &prepare);
    result(
        &store,
        &prepare,
        CommandOutput::Workspace {
            workspace: Workspace {
                path: "/work".into(),
                branch: s.branch,
                base_commit: "a".repeat(40),
            },
        },
    );
    let cp = store
        .enqueue(
            s.id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: rev(&store, s.id),
                kind: CommandKind::Checkpoint,
                prompt: None,
            },
        )
        .unwrap();
    ack(&store, &cp);
    let receipt = store
        .upload_checkpoint_bundle(s.runner_id, cp.id, 1, b"fixture".to_vec())
        .unwrap();
    result(
        &store,
        &cp,
        CommandOutput::Checkpoint {
            checkpoint: GitCheckpoint {
                commit: "b".repeat(40),
                parent: "a".repeat(40),
                tree: "c".repeat(40),
                bundle_path: "/bundle".into(),
                bundle_sha256: receipt.sha256,
                bundle_bytes: receipt.bytes,
                manifest_path: "/manifest".into(),
            },
        },
    );
    let request = RequestReview {
        request_id: RequestId::new(),
        expected_revision: rev(&store, s.id),
        checkpoint_command: cp.id,
        expected_target_commit: "d".repeat(40),
    };
    let reviewed = store.request_review(s.id, request.clone()).unwrap();
    assert_eq!(store.request_review(s.id, request).unwrap(), reviewed);
    assert!(store
        .require_review_support(s.runner_id, reviewed.id, 1, false)
        .is_err());
    ack(&store, &reviewed);
    let job = store
        .runner_review_job(s.runner_id, reviewed.id, 1)
        .unwrap();
    assert!(
        matches!(&job,ReviewJob::Review {test_command,target_commit,..} if test_command=="python3 -m unittest" && target_commit==&"d".repeat(40))
    );
    let report = ReviewResult {
        checkpoint: cp.id,
        target_commit: "d".repeat(40),
        candidate_commit: "e".repeat(40),
        tree: "f".repeat(40),
        image: format!("sha256:{}", "a".repeat(64)),
        test_exit_code: 0,
        removal: RemovalProof {
            runtime: RuntimeRef {
                container_id: "b".repeat(64),
                started_at: "2026-11-22T00:00:00Z".into(),
            },
            daemon_id: "original".into(),
            observed_unix_ms: 1,
        },
        log_path: "/private/test.log".into(),
    };
    let mut wrong = report.clone();
    wrong.test_exit_code = 1;
    assert!(store
        .complete(
            s.runner_id,
            reviewed.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Review { review: wrong }
            }
        )
        .is_err());
    assert!(store
        .request_integration(
            s.id,
            RequestIntegration {
                request_id: RequestId::new(),
                expected_revision: rev(&store, s.id),
                review_command: reviewed.id
            }
        )
        .is_err());
    let hosted = ReportHostedReview {
        request_id: RequestId::new(),
        ownership_epoch: 1,
        result: HostedReview {
            review_command: reviewed.id,
            candidate_commit: report.candidate_commit.clone(),
            target_branch: "main".into(),
            publication_branch: format!("yoloforge/review-{}", reviewed.id),
            repository: "owner/repo".into(),
            url: "https://github.com/owner/repo/pull/1".into(),
        },
    };
    assert!(store
        .report_hosted_review(reviewed.id, hosted.clone())
        .is_err());
    result(
        &store,
        &reviewed,
        CommandOutput::Review {
            review: report.clone(),
        },
    );
    let mut wrong_hosted = hosted.clone();
    wrong_hosted.request_id = RequestId::new();
    wrong_hosted.result.candidate_commit = "f".repeat(40);
    assert!(store
        .report_hosted_review(reviewed.id, wrong_hosted)
        .is_err());
    assert_eq!(
        store
            .report_hosted_review(reviewed.id, hosted.clone())
            .unwrap(),
        hosted.result
    );
    assert_eq!(
        Store::open(&path)
            .unwrap()
            .report_hosted_review(reviewed.id, hosted.clone())
            .unwrap(),
        hosted.result
    );
    let mut retarget = hosted.clone();
    retarget.request_id = RequestId::new();
    retarget.result.url = "https://github.com/owner/repo/pull/2".into();
    assert!(store.report_hosted_review(reviewed.id, retarget).is_err());
    let request = RequestIntegration {
        request_id: RequestId::new(),
        expected_revision: rev(&store, s.id),
        review_command: reviewed.id,
    };
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_review BEFORE INSERT ON reviews BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
    assert!(store.request_integration(s.id, request.clone()).is_err());
    assert_eq!(rev(&store, s.id), request.expected_revision);
    db.execute_batch("DROP TRIGGER fail_review").unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let threads = (0..2)
        .map(|_| {
            let barrier = barrier.clone();
            let path = path.clone();
            let mut r = request.clone();
            r.request_id = RequestId::new();
            std::thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                barrier.wait();
                store.request_integration(s.id, r)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let mut successes = threads
        .into_iter()
        .filter_map(|t| t.join().unwrap().ok())
        .collect::<Vec<_>>();
    assert_eq!(successes.len(), 1);
    let integration = successes.pop().unwrap();
    ack(&store, &integration);
    assert!(store
        .runner_review_job(RunnerId::new(), integration.id, 1)
        .is_err());
    let wrong = IntegrationResult {
        review_command: reviewed.id,
        target_branch: "main".into(),
        previous_commit: "d".repeat(40),
        commit: "9".repeat(40),
    };
    assert!(store
        .complete(
            s.runner_id,
            integration.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Integration { integration: wrong }
            }
        )
        .is_err());
    result(
        &store,
        &integration,
        CommandOutput::Integration {
            integration: IntegrationResult {
                review_command: reviewed.id,
                target_branch: "main".into(),
                previous_commit: report.target_commit,
                commit: report.candidate_commit,
            },
        },
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.review_job(reviewed.id).unwrap(), job);
}
