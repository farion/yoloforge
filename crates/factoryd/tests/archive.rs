use factory_core::*;
use factory_protocol::*;
use factoryd::store::{Store, StoreError};
use serde_json::json;

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
