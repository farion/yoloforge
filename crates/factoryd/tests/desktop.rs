use factory_core::*;
use factory_protocol::*;
use factoryd::store::{Store, StoreError};

fn project(store: &Store) -> Project {
    store
        .create_project(CreateProject {
            runner_mapping: None,
            key: None,
            request_id: RequestId::new(),
            name: "Desktop".into(),
            repository: "factory:managed".into(),
            base_branch: "main".into(),
        })
        .unwrap()
}
fn submission(project: ProjectId, draft: bool) -> SubmitTask {
    SubmitTask {
        edit_token: None,
        task: SaveTask {
            requested_session: None,
            request_id: RequestId::new(),
            id: None,
            expected_revision: Revision(0),
            project_id: project,
            prompt: "Bitte diese Datei prüfen".into(),
            images: vec![],
        },
        draft,
        files: vec![PromptFile::from_bytes("notizen.txt".into(), "Grüße\n".as_bytes()).unwrap()],
    }
}

#[test]
fn task_edit_is_exclusive_durable_replayable_and_blocks_scheduling_until_release() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edit.sqlite");
    let store = Store::open(&path).unwrap();
    let p = project(&store);
    let original = store.submit_task(submission(p.id, false)).unwrap();
    assert!(store
        .snapshot()
        .unwrap()
        .editable_tasks
        .contains_key(&original.id));
    let begin = BeginTaskEdit {
        request_id: RequestId::new(),
        expected_revision: original.revision,
        edit_token: None,
    };
    let edit = store.begin_task_edit(original.id, begin.clone()).unwrap();
    assert!(edit.ticket.draft);
    assert_eq!(edit.ticket.created_unix_ms, original.created_unix_ms);
    assert_eq!(edit.ticket.files, original.files);
    assert_eq!(
        store
            .begin_task_edit(original.id, begin.clone())
            .unwrap()
            .edit_token,
        edit.edit_token
    );
    let wrong = BeginTaskEdit {
        request_id: RequestId::new(),
        expected_revision: edit.ticket.revision,
        edit_token: None,
    };
    assert!(store.begin_task_edit(original.id, wrong).is_err());
    let mut release = submission(p.id, false);
    release.task.id = Some(original.id);
    release.task.expected_revision = edit.ticket.revision;
    assert!(
        store.submit_task(release.clone()).is_err(),
        "A different client must not release a held draft"
    );
    store.autostart_runner(RunnerId::new()).unwrap();
    store.scheduler_tick().unwrap();
    assert!(store.snapshot().unwrap().commands.is_empty());
    assert!(store
        .create_stream(CreateStream {
            request_id: RequestId::new(),
            project_id: p.id,
            goal: "Bypass".into(),
            runner_id: RunnerId::new(),
            ticket_ids: vec![original.id]
        })
        .is_err());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .begin_task_edit(original.id, begin)
            .unwrap()
            .edit_token,
        edit.edit_token
    );
    let resumed = store
        .begin_task_edit(
            original.id,
            BeginTaskEdit {
                request_id: RequestId::new(),
                expected_revision: edit.ticket.revision,
                edit_token: Some(edit.edit_token),
            },
        )
        .unwrap();
    assert_eq!(resumed.ticket, edit.ticket);
    release.edit_token = Some(edit.edit_token);
    let released = store.submit_task(release.clone()).unwrap();
    assert!(!released.draft);
    assert_eq!(store.submit_task(release).unwrap(), released);
    store.scheduler_tick().unwrap();
    assert!(!store.snapshot().unwrap().commands.is_empty());
}

#[test]
fn tasks_cannot_be_edited_after_orchestrator_processing_even_without_a_work_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("routing.sqlite")).unwrap();
    let p = project(&store);
    let original = store.submit_task(submission(p.id, false)).unwrap();
    store.autostart_runner(RunnerId::new()).unwrap();
    store.scheduler_tick().unwrap();
    let snapshot = store.snapshot().unwrap();
    let processed = &snapshot.tickets[0];
    assert!(
        processed.execution_command.is_none(),
        "This regression must cover processing before work-session assignment"
    );
    assert!(!snapshot.commands.is_empty());
    assert!(!snapshot.editable_tasks.contains_key(&original.id));
    assert!(store
        .begin_task_edit(
            original.id,
            BeginTaskEdit {
                request_id: RequestId::new(),
                expected_revision: processed.revision,
                edit_token: None
            }
        )
        .is_err());
    let mut save = submission(p.id, true);
    save.task.id = Some(processed.id);
    save.task.expected_revision = processed.revision;
    assert!(store.submit_task(save).is_err());
    assert!(
        !store.snapshot().unwrap().tickets[0].draft,
        "A failed edit must not stop existing work"
    );
}

#[test]
fn a_draft_can_be_edited_again_after_saving_without_releasing_work() {
    let store = Store::open(std::path::Path::new(":memory:")).unwrap();
    let p = project(&store);
    let original = store.submit_task(submission(p.id, true)).unwrap();
    let edit = store
        .begin_task_edit(
            original.id,
            BeginTaskEdit {
                request_id: RequestId::new(),
                expected_revision: original.revision,
                edit_token: None,
            },
        )
        .unwrap();
    let mut save = submission(p.id, true);
    save.edit_token = Some(edit.edit_token);
    save.task.id = Some(original.id);
    save.task.expected_revision = edit.ticket.revision;
    save.task.prompt = "Geänderter Auftrag".into();
    let saved = store.submit_task(save).unwrap();
    assert!(saved.draft);
    let next = store
        .begin_task_edit(
            original.id,
            BeginTaskEdit {
                request_id: RequestId::new(),
                expected_revision: saved.revision,
                edit_token: None,
            },
        )
        .unwrap();
    assert_ne!(edit.edit_token, next.edit_token);
    assert_eq!(next.ticket.prompt.as_deref(), Some("Geänderter Auftrag"));
}

#[test]
fn project_deletion_requires_confirmation_is_replay_safe_and_preserves_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("delete.sqlite");
    let store = Store::open(&path).unwrap();
    let p = project(&store);
    let ticket = store.submit_task(submission(p.id, true)).unwrap();
    let body = DeleteProject {
        request_id: RequestId::new(),
        expected_revision: p.revision,
        confirm_name: p.name.clone(),
    };
    let mut invalid = body.clone();
    invalid.confirm_name = "falsch".into();
    assert!(store.delete_project(p.id, invalid).is_err());
    let mut stale = body.clone();
    stale.expected_revision = Revision(999);
    assert!(matches!(
        store.delete_project(p.id, stale),
        Err(StoreError::RevisionConflict(_))
    ));
    let deleted = store.delete_project(p.id, body.clone()).unwrap();
    assert!(deleted.deleted && deleted.paused);
    assert!(store.snapshot().unwrap().projects.is_empty());
    assert_eq!(store.snapshot().unwrap().tickets[0], ticket);
    assert!(matches!(
        store.submit_task(submission(p.id, true)),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.pause_project(
            p.id,
            PauseProject {
                request_id: RequestId::new(),
                expected_revision: deleted.revision,
                paused: false
            }
        ),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.create_ticket(CreateTicket {
            request_id: RequestId::new(),
            project_id: p.id,
            title: "Nicht zulässig".into(),
            description: String::new(),
            acceptance_criteria: String::new(),
            priority: 1
        }),
        Err(StoreError::NotFound)
    ));
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.delete_project(p.id, body).unwrap(), deleted);
    assert!(store.snapshot().unwrap().projects.is_empty());
    assert_eq!(store.snapshot().unwrap().tickets[0], ticket);
    store.autostart_runner(RunnerId::new()).unwrap();
    store.scheduler_tick().unwrap();
    assert!(store.snapshot().unwrap().commands.is_empty());
}

#[test]
fn project_deletion_never_stops_or_hides_queued_work() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("busy.sqlite")).unwrap();
    let p = project(&store);
    let stream = store
        .create_stream(CreateStream {
            request_id: RequestId::new(),
            project_id: p.id,
            goal: "Arbeit behalten".into(),
            runner_id: RunnerId::new(),
            ticket_ids: vec![],
        })
        .unwrap();
    store
        .enqueue(
            stream.id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: stream.revision,
                kind: CommandKind::PrepareWorkspace,
                prompt: None,
            },
        )
        .unwrap();
    assert!(store
        .delete_project(
            p.id,
            DeleteProject {
                request_id: RequestId::new(),
                expected_revision: p.revision,
                confirm_name: p.name.clone()
            }
        )
        .is_err());
    let snapshot = store.snapshot().unwrap();
    assert!(!snapshot.projects[0].deleted);
    assert_eq!(snapshot.commands.len(), 1);
    assert_eq!(snapshot.commands[0].state, CommandState::Pending);
}

#[test]
fn drafts_survive_restart_cannot_be_scheduled_or_manually_assigned_and_release_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let p = project(&store);
    let runner = RunnerId::new();
    let draft = submission(p.id, true);
    let ticket = store.submit_task(draft.clone()).unwrap();
    assert!(ticket.draft);
    assert_eq!(store.submit_task(draft).unwrap(), ticket);
    store.autostart_runner(runner).unwrap();
    store.scheduler_tick().unwrap();
    assert!(store.snapshot().unwrap().commands.is_empty());
    assert!(matches!(
        store.create_stream(CreateStream {
            request_id: RequestId::new(),
            project_id: p.id,
            goal: "Bypass".into(),
            runner_id: runner,
            ticket_ids: vec![ticket.id]
        }),
        Err(StoreError::Invalid(_))
    ));
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot().unwrap().tickets[0], ticket);
    let mut release = submission(p.id, false);
    release.task.id = Some(ticket.id);
    release.task.expected_revision = ticket.revision;
    let released = store.submit_task(release.clone()).unwrap();
    assert!(!released.draft);
    assert_eq!(released.files, ticket.files);
    assert_eq!(store.submit_task(release).unwrap(), released);
}

#[test]
fn explicit_scheduler_pause_is_not_undone_by_desktop_task_submission() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let p = project(&store);
    let runner = RunnerId::new();
    store
        .set_scheduler(
            p.id,
            SetScheduler {
                request_id: RequestId::new(),
                expected_revision: Revision(0),
                enabled: false,
                runner_ids: vec![runner],
                max_parallel: 1,
                start_budget: 12,
            },
        )
        .unwrap();
    store.submit_task(submission(p.id, false)).unwrap();
    assert!(!store.scheduler_policy(p.id).unwrap().unwrap().enabled);
    store.scheduler_tick().unwrap();
    assert!(store.snapshot().unwrap().commands.is_empty());
}

#[test]
fn legacy_task_replay_and_new_submission_have_distinct_fingerprints() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let p = project(&store);
    let body = submission(p.id, false).task;
    let old = store.save_task(body.clone()).unwrap();
    assert_eq!(store.save_task(body.clone()).unwrap(), old);
    assert!(matches!(
        store.submit_task(SubmitTask {
            edit_token: None,
            task: body,
            draft: true,
            files: vec![]
        }),
        Err(StoreError::RequestConflict)
    ));
}

#[test]
fn history_pages_are_complete_stable_and_newest_first_without_cursor_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let p = project(&store);
    for i in 0..121 {
        store
            .create_ticket(CreateTicket {
                request_id: RequestId::new(),
                project_id: p.id,
                title: format!("Ticket {i}"),
                description: String::new(),
                acceptance_criteria: String::new(),
                priority: 1,
            })
            .unwrap();
    }
    let first = store.event_history(None).unwrap();
    assert_eq!(first.events.len(), 100);
    let latest = first.events.last().unwrap().sequence;
    assert_eq!(latest, store.snapshot().unwrap().cursor);
    let before = first.next_before.unwrap();
    let older = store.event_history(Some(before)).unwrap();
    assert!(older.next_before.is_none());
    assert_eq!(older.events.len(), 22);
    assert!(older.events.iter().all(|e| e.sequence < before));
    assert_eq!(
        older.events.last().unwrap().sequence + 1,
        first.events[0].sequence
    );
    assert_eq!(store.events(latest, 100).unwrap().events.len(), 0);
}

#[tokio::test]
async fn presence_is_registration_fenced_scoped_and_not_recovered_as_live_after_server_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let token = "desktop-test-token-at-least-thirty-two-bytes";
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = factoryd::router(token, store.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client =
        factory_protocol::client::FactoryClient::new(&format!("http://{address}"), token).unwrap();
    let runner = RunnerId::new();
    let registration = client
        .register(&RegisterRunner {
            capabilities: vec![],
            id: runner,
            instance_id: InstanceId::new(),
            name: "Desktop runner".into(),
            location: RunnerLocation::Local,
            version: "test".into(),
            opencode: OpenCodeStatus::Ready {
                version: "test".into(),
            },
        })
        .await
        .unwrap();
    let report = ReportInstances {
        instances: vec![OpenCodeInstance {
            id: "a".repeat(64),
            runner_id: runner,
            commands: vec![],
            observed_unix_ms: u64::MAX,
        }],
    };
    assert!(client
        .report_instances(runner, RegistrationId::new(), &report)
        .await
        .is_err());
    client
        .report_instances(runner, registration.registration_id, &report)
        .await
        .unwrap();
    let instances = client.instances().await.unwrap();
    assert_eq!(instances.len(), 1);
    assert_ne!(instances[0].observed_unix_ms, u64::MAX);
    let mut foreign = report;
    foreign.instances[0].commands.push(CommandId::new());
    assert!(client
        .report_instances(runner, registration.registration_id, &foreign)
        .await
        .is_err());
    assert_eq!(client.instances().await.unwrap().len(), 1);
    server.abort();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = factoryd::router(token, store);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client =
        factory_protocol::client::FactoryClient::new(&format!("http://{address}"), token).unwrap();
    assert!(client.instances().await.unwrap().is_empty());
    server.abort();
}
