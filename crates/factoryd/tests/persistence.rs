use factory_core::*;
use factory_protocol::*;
use factoryd::store::{Store, StoreError};
use std::sync::{Arc, Barrier};

fn project() -> CreateProject {
    CreateProject {
        logo: None,
        runner_mapping: None,
        key: None,
        request_id: RequestId::new(),
        name: "Factory".into(),
        repository: "git@github.com:example/factory.git".into(),
        base_branch: "main".into(),
    }
}
fn ticket(project_id: ProjectId) -> CreateTicket {
    CreateTicket {
        request_id: RequestId::new(),
        project_id,
        title: "Persist work".into(),
        description: "Keep state\nafter restart".into(),
        acceptance_criteria: "No duplicate".into(),
        priority: 2,
    }
}
fn update(title: &str) -> UpdateTicket {
    UpdateTicket {
        request_id: RequestId::new(),
        expected_revision: Revision(1),
        title: Some(title.into()),
        description: None,
        acceptance_criteria: None,
        priority: None,
        status: Some(TicketState::Ready),
    }
}

#[test]
fn project_metadata_is_durable_validated_and_cannot_retarget_existing_streams() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("metadata.db");
    let store = Store::open(&path).unwrap();
    let p = store.create_project(project()).unwrap();
    let body = UpdateProject {
        logo: None,
        runner_mapping: None,
        key: None,
        request_id: RequestId::new(),
        expected_revision: p.revision,
        name: Some("Renamed".into()),
        repository: Some("/new/repo".into()),
        base_branch: Some("develop".into()),
    };
    let edited = store.update_project(p.id, body.clone()).unwrap();
    assert_eq!(edited.revision, Revision(2));
    assert_eq!(edited.profile, p.profile);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.update_project(p.id, body.clone()).unwrap(), edited);
    let stream = store
        .create_stream(CreateStream {
            request_id: RequestId::new(),
            project_id: p.id,
            goal: "pinned".into(),
            runner_id: RunnerId::new(),
            ticket_ids: vec![],
        })
        .unwrap();
    let before = store.snapshot().unwrap().cursor;
    for (repository, base_branch) in [(Some("/other".into()), None), (None, Some("other".into()))] {
        assert!(matches!(
            store.update_project(
                p.id,
                UpdateProject {
                    logo: None,
                    runner_mapping: None,
                    key: None,
                    request_id: RequestId::new(),
                    expected_revision: edited.revision,
                    name: None,
                    repository,
                    base_branch
                }
            ),
            Err(StoreError::WorkConflict)
        ));
    }
    assert_eq!(store.snapshot().unwrap().cursor, before);
    let renamed = store
        .update_project(
            p.id,
            UpdateProject {
                logo: None,
                runner_mapping: None,
                key: None,
                request_id: RequestId::new(),
                expected_revision: edited.revision,
                name: Some("Still renameable".into()),
                repository: None,
                base_branch: None,
            },
        )
        .unwrap();
    assert_eq!(renamed.repository, "/new/repo");
    assert_eq!(store.snapshot().unwrap().streams[0], stream);
    assert_eq!(
        store.update_project(p.id, body).unwrap(),
        edited,
        "old request replays its original response even after streams exist"
    );
    let other = store.create_project(project()).unwrap();
    let cursor = store.snapshot().unwrap().cursor;
    for repository in [
        "https://user:password@example.org/repo",
        "https://example.org/repo?token=secret",
        "",
    ] {
        assert!(matches!(
            store.update_project(
                other.id,
                UpdateProject {
                    logo: None,
                    runner_mapping: None,
                    key: None,
                    request_id: RequestId::new(),
                    expected_revision: other.revision,
                    name: None,
                    repository: Some(repository.into()),
                    base_branch: None
                }
            ),
            Err(StoreError::Invalid(_))
        ));
    }
    assert_eq!(store.snapshot().unwrap().cursor, cursor);
}

#[test]
fn a_local_project_gets_a_runner_only_when_automatic_setup_is_needed() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("automatic.db")).unwrap();
    let mut create = project();
    create.repository = "/path/to/local/project".into();
    create.base_branch = "develop".into();
    let project = store.create_project(create).unwrap();
    assert!(project.setup.is_none());
    assert!(project.runner_bindings.is_empty());
    let runner = RunnerId::new();
    store.autostart_runner(runner).unwrap();
    assert!(store.scheduler_policy(project.id).unwrap().is_none());
    store.create_ticket(ticket(project.id)).unwrap();
    store.autostart_runner(runner).unwrap();
    let snapshot = store.snapshot().unwrap();
    let configured = &snapshot.projects[0];
    let setup = configured.setup.as_ref().unwrap();
    assert_eq!(setup.runner_id, runner);
    assert_eq!(setup.repository_source, project.repository);
    assert_eq!(configured.base_branch, "develop");
    assert!(snapshot.streams.is_empty());
}

#[test]
fn project_runner_selection_persists_and_can_return_to_automatic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mapping.db");
    let store = Store::open(&path).unwrap();
    let manual = RunnerId::new();
    let automatic = RunnerId::new();
    let mut body = project();
    body.repository = "/local/repo".into();
    body.runner_mapping = Some(ProjectRunnerMapping::Runner(manual));
    let project = store.create_project(body).unwrap();
    store.create_ticket(ticket(project.id)).unwrap();
    store.autostart_runner(automatic).unwrap();
    assert!(store.scheduler_policy(project.id).unwrap().is_none());
    store.autostart_runner(manual).unwrap();
    assert_eq!(store.scheduler_policy(project.id).unwrap().unwrap().runner_ids, [manual]);
    drop(store);

    let store = Store::open(&path).unwrap();
    assert_eq!(store.snapshot().unwrap().projects[0].runner_mapping, Some(ProjectRunnerMapping::Runner(manual)));
    let change = UpdateProject {
        logo: None,
        runner_mapping: Some(ProjectRunnerMapping::Automatic),
        key: None,
        request_id: RequestId::new(),
        expected_revision: project.revision,
        name: None,
        repository: None,
        base_branch: None,
    };
    let edited = store.update_project(project.id, change.clone()).unwrap();
    assert_eq!(edited.runner_mapping, Some(ProjectRunnerMapping::Automatic));
    assert!(store.scheduler_policy(project.id).unwrap().unwrap().runner_ids.is_empty());
    store.scheduler_tick().unwrap();
    store.autostart_runner(automatic).unwrap();
    let policy = store.scheduler_policy(project.id).unwrap().unwrap();
    assert_eq!(policy.runner_ids, [automatic]);
    let snapshot = store.snapshot().unwrap();
    let configured = &snapshot.projects[0];
    assert_eq!(configured.setup.as_ref().unwrap().runner_id, automatic);
    assert!(configured.runner_bindings.contains_key(&manual));
    assert!(snapshot.streams.is_empty());
    assert_eq!(store.update_project(project.id, change).unwrap(), edited);
}

#[test]
fn profile_and_metadata_share_one_project_revision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("race.db");
    let first = Store::open(&path).unwrap();
    let second = Store::open(&path).unwrap();
    let p = first.create_project(project()).unwrap();
    let gate = Arc::new(Barrier::new(2));
    let other = gate.clone();
    let id = p.id;
    let writer = std::thread::spawn(move || {
        other.wait();
        first.update_project(
            id,
            UpdateProject {
                logo: None,
                runner_mapping: None,
                key: None,
                request_id: RequestId::new(),
                expected_revision: Revision(1),
                name: Some("new name".into()),
                repository: None,
                base_branch: None,
            },
        )
    });
    gate.wait();
    let profile = second.update_profile(
        id,
        UpdateProfile {
            request_id: RequestId::new(),
            expected_revision: Revision(1),
            profile: ExecutionProfile {
                provider_id: "test".into(),
                model_id: "test".into(),
                ..Default::default()
            },
        },
    );
    let metadata = writer.join().unwrap();
    assert_ne!(metadata.is_ok(), profile.is_ok());
    assert!(matches!(
        metadata.as_ref().err().or(profile.as_ref().err()),
        Some(StoreError::RevisionConflict(Revision(2)))
    ));
    assert_eq!(second.snapshot().unwrap().projects[0].revision, Revision(2));
}

#[test]
fn durable_replay_and_cursor_survive_reopening_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("factory.db");
    let store = Store::open(&path).unwrap();
    let create = project();
    let p = store.create_project(create.clone()).unwrap();
    let create_ticket = ticket(p.id);
    let t = store.create_ticket(create_ticket.clone()).unwrap();
    let change = update("Ready for work");
    let edited = store.update_ticket(t.id, change.clone()).unwrap();
    assert_eq!(edited.description, t.description);
    let snap = store.snapshot().unwrap();
    assert_eq!(snap.cursor, 3);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.create_project(create.clone()).unwrap(), p);
    assert_eq!(store.create_ticket(create_ticket.clone()).unwrap(), t);
    // Old expected_revision remains valid for an identical replay of an already committed request.
    assert_eq!(store.update_ticket(t.id, change.clone()).unwrap(), edited);
    let mut conflicting = change;
    conflicting.title = Some("different".into());
    assert!(matches!(
        store.update_ticket(t.id, conflicting),
        Err(StoreError::RequestConflict)
    ));
    assert!(matches!(
        store.update_ticket(t.id, update("stale")),
        Err(StoreError::RevisionConflict(Revision(2)))
    ));
    let snap = store.snapshot().unwrap();
    assert_eq!(snap.cursor, 3);
    assert_eq!(snap.tickets, vec![edited]);
    assert_eq!(snap.projects, vec![p]);
    let page = store.events(0, 2).unwrap();
    assert_eq!(page.events.len(), 2);
    assert_eq!(page.next_cursor, 2);
    assert_eq!(store.events(page.next_cursor, 100).unwrap().events.len(), 1);
    assert!(store.events(snap.cursor, 100).unwrap().events.is_empty());
}

#[test]
fn independent_connections_serialize_simultaneous_edits_and_same_request_retries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("factory.db");
    let first = Store::open(&path).unwrap();
    let second = Store::open(&path).unwrap();
    let request = project();
    let retry = request.clone();
    let gate = Arc::new(Barrier::new(2));
    let a_gate = gate.clone();
    let a = first.clone();
    let task = std::thread::spawn(move || {
        a_gate.wait();
        a.create_project(request).unwrap()
    });
    gate.wait();
    let p = second.create_project(retry).unwrap();
    assert_eq!(task.join().unwrap(), p);
    let t = first.create_ticket(ticket(p.id)).unwrap();
    let gate = Arc::new(Barrier::new(2));
    let a_gate = gate.clone();
    let task = std::thread::spawn(move || {
        a_gate.wait();
        first.update_ticket(t.id, update("A"))
    });
    gate.wait();
    let b = second.update_ticket(t.id, update("B"));
    let a = task.join().unwrap();
    assert!(matches!(
        (&a, &b),
        (Ok(_), Err(StoreError::RevisionConflict(_)))
            | (Err(StoreError::RevisionConflict(_)), Ok(_))
    ));
    assert_eq!(second.snapshot().unwrap().cursor, 3);
}

#[test]
fn failed_event_write_rolls_back_entity_and_request_journal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("factory.db");
    let store = Store::open(&path).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    let request = project();
    assert!(matches!(
        store.create_project(request.clone()),
        Err(StoreError::Internal)
    ));
    assert!(store.snapshot().unwrap().projects.is_empty());
    let count: u64 = connection
        .query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    connection.execute_batch("DROP TRIGGER fail_event").unwrap();
    store.create_project(request).unwrap();
    assert_eq!(store.snapshot().unwrap().cursor, 1);
}

#[test]
fn rejects_newer_schema_missing_project_and_embedded_http_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA user_version=99").unwrap();
    assert!(Store::open(&path).is_err());
    let store = Store::open(&dir.path().join("valid.db")).unwrap();
    assert!(matches!(
        store.create_ticket(ticket(ProjectId::new())),
        Err(StoreError::NotFound)
    ));
    let mut body = project();
    body.repository = "https://user:secret@example.org/repo.git".into();
    assert!(matches!(
        store.create_project(body),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(store.snapshot().unwrap().cursor, 0);
}
