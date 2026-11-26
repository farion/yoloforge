use factory_core::*;
use factory_protocol::*;
use factoryd::store::Store;

fn fixture(path: &std::path::Path) -> (Store, RunnerCommand) {
    let store = Store::open(path).unwrap();
    let project = store
        .create_project(CreateProject {
            request_id: RequestId::new(),
            name: "Restart".into(),
            repository: "/fixture".into(),
            base_branch: "main".into(),
            key: None,
            logo: None,
            runner_mapping: None,
        })
        .unwrap();
    store
        .update_profile(
            project.id,
            UpdateProfile {
                request_id: RequestId::new(),
                expected_revision: project.revision,
                profile: ExecutionProfile {
                    provider_id: "openai".into(),
                    model_id: "test".into(),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    store
        .create_ticket(CreateTicket {
            request_id: RequestId::new(),
            project_id: project.id,
            title: "Queued work".into(),
            description: "Preserve this task".into(),
            acceptance_criteria: "Tests pass".into(),
            priority: 1,
        })
        .unwrap();
    let runner = RunnerId::new();
    store
        .set_scheduler(
            project.id,
            SetScheduler {
                request_id: RequestId::new(),
                expected_revision: Revision(0),
                enabled: true,
                runner_ids: vec![runner],
                max_parallel: 1,
            },
        )
        .unwrap();
    store.scheduler_tick().unwrap();
    let command = store.pending(runner).unwrap().pop().unwrap();
    (store, command)
}

fn ack(command: &RunnerCommand) -> AckCommand {
    AckCommand {
        request_id: RequestId::new(),
        ownership_epoch: command.ownership_epoch,
    }
}

#[test]
fn restart_hold_fences_queued_work_survives_server_restart_and_does_not_remove_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (store, command) = fixture(&path);
    let request = RequestId::new();
    store
        .hold_runner_restart(command.runner_id, request)
        .unwrap();
    store
        .hold_runner_restart(command.runner_id, request)
        .unwrap();
    assert!(store
        .acknowledge(command.runner_id, command.id, ack(&command))
        .is_err());
    assert_eq!(store.pending(command.runner_id).unwrap()[0].id, command.id);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(store
        .acknowledge(command.runner_id, command.id, ack(&command))
        .is_err());
    assert!(store
        .release_runner_restart(command.runner_id, RequestId::new())
        .is_err());
    store
        .release_runner_restart(command.runner_id, request)
        .unwrap();
    assert_eq!(
        store
            .acknowledge(command.runner_id, command.id, ack(&command))
            .unwrap()
            .state,
        CommandState::Accepted
    );
    assert!(
        store
            .hold_runner_restart(command.runner_id, RequestId::new())
            .is_err(),
        "Accepted work cannot be interrupted by a restart"
    );
}

#[test]
fn expired_hold_recovers_from_a_desktop_crash_and_stale_release_cannot_clear_a_new_hold() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (store, command) = fixture(&path);
    let old = RequestId::new();
    store.hold_runner_restart(command.runner_id, old).unwrap();
    assert!(store
        .hold_runner_restart(command.runner_id, RequestId::new())
        .is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE runner_restart_holds SET expires_ms=0", [])
        .unwrap();
    let new = RequestId::new();
    store.hold_runner_restart(command.runner_id, new).unwrap();
    assert!(store
        .release_runner_restart(command.runner_id, old)
        .is_err());
    assert!(store
        .acknowledge(command.runner_id, command.id, ack(&command))
        .is_err());
    db.execute("UPDATE runner_restart_holds SET expires_ms=0", [])
        .unwrap();
    assert!(store
        .acknowledge(command.runner_id, command.id, ack(&command))
        .is_ok());
}

#[test]
fn concurrent_claim_and_restart_hold_have_exactly_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (store, command) = fixture(&path);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other = Store::open(&path).unwrap();
    let c = command.clone();
    let gate = barrier.clone();
    let claim = std::thread::spawn(move || {
        gate.wait();
        other.acknowledge(c.runner_id, c.id, ack(&c)).is_ok()
    });
    barrier.wait();
    let held = store
        .hold_runner_restart(command.runner_id, RequestId::new())
        .is_ok();
    assert_ne!(
        held,
        claim.join().unwrap(),
        "Restart and command execution must not race"
    );
}

#[tokio::test]
async fn restart_api_requires_admin_and_reports_the_exact_startup_instance() {
    let dir = tempfile::tempdir().unwrap();
    let (store, command) = fixture(&dir.path().join("db"));
    let token = "restart-test-only-0123456789abcdef0123456789";
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let router = factoryd::router(token, store.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let body = RunnerRestartHold {
        request_id: RequestId::new(),
    };
    let response = reqwest::Client::new()
        .put(format!("{url}/v1/runners/{}/restart", command.runner_id))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let client = factory_protocol::client::FactoryClient::new(&url, token).unwrap();
    for role in [DeviceRole::Operator, DeviceRole::Runner] {
        let credential = client
            .create_device(&CreateDevice {
                name: "Non-admin".into(),
                role,
                runner_id: (role == DeviceRole::Runner).then_some(command.runner_id),
            })
            .await
            .unwrap();
        let restricted =
            factory_protocol::client::FactoryClient::new(&url, &credential.token).unwrap();
        assert!(matches!(
            restricted
                .hold_runner_restart(command.runner_id, body.request_id)
                .await,
            Err(factory_protocol::client::ClientError::Http {
                status: reqwest::StatusCode::FORBIDDEN,
                ..
            })
        ));
    }
    let instance = InstanceId::new();
    client
        .register(&RegisterRunner {
            id: command.runner_id,
            instance_id: instance,
            name: "Local".into(),
            location: RunnerLocation::Local,
            version: "test".into(),
            capabilities: vec![],
            opencode: OpenCodeStatus::Ready {
                version: "test".into(),
            },
        })
        .await
        .unwrap();
    let status = client
        .hold_runner_restart(command.runner_id, body.request_id)
        .await
        .unwrap();
    assert_eq!(status.instance_id, Some(instance));
    assert!(status.ready);
    assert!(store
        .acknowledge(command.runner_id, command.id, ack(&command))
        .is_err());
    assert_eq!(
        client
            .runner_restart_status(command.runner_id)
            .await
            .unwrap()
            .instance_id,
        Some(instance)
    );
    client
        .release_runner_restart(command.runner_id, body.request_id)
        .await
        .unwrap();
    assert!(store
        .acknowledge(command.runner_id, command.id, ack(&command))
        .is_ok());
    server.abort();
}
