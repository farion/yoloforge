use factory_core::*;
use factory_protocol::*;
use reqwest::{Client, StatusCode};

const TOKEN: &str = "test-only-credential-0123456789abcdef";

#[tokio::test]
async fn task_edit_endpoint_reserves_full_payload_and_requires_owner_to_release() {
    use factory_protocol::client::FactoryClient;
    let server = server().await;
    let client = FactoryClient::new(&server.url, TOKEN).unwrap();
    let project:Project=serde_json::from_value(client.management_request("POST","/v1/projects",&serde_json::json!({"request_id":RequestId::new(),"name":"Bearbeiten","repository":"factory:managed","base_branch":"main"})).await.unwrap()).unwrap();
    let file = PromptFile::from_bytes("grüße.txt".into(), "Vollständige Daten".as_bytes()).unwrap();
    let mut submission = SubmitTask {
        edit_token: None,
        task: SaveTask {
            request_id: RequestId::new(),
            id: None,
            expected_revision: Revision(0),
            project_id: project.id,
            prompt: "Noch nicht verarbeitet".into(),
            images: vec![],
            requested_session: None,
        },
        draft: false,
        files: vec![file.clone()],
    };
    let ticket: Ticket = serde_json::from_value(
        client
            .management_request(
                "POST",
                "/v1/tasks/submit",
                &serde_json::to_value(&submission).unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let snapshot = client.desktop_snapshot().await.unwrap();
    assert_eq!(
        snapshot.editable_tasks.get(&ticket.id),
        Some(&ticket.revision)
    );
    assert!(snapshot.tickets[0].files[0].data.is_empty());
    let body = BeginTaskEdit {
        request_id: RequestId::new(),
        expected_revision: ticket.revision,
        edit_token: None,
    };
    let edit: TaskEdit = serde_json::from_value(
        client
            .management_request(
                "POST",
                &format!("/v1/tasks/{}/edit", ticket.id),
                &serde_json::to_value(&body).unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(edit.edit_token, body.request_id);
    assert_eq!(edit.ticket.files, vec![file]);
    assert!(edit.ticket.draft);
    submission.task.id = Some(ticket.id);
    submission.task.request_id = RequestId::new();
    submission.task.expected_revision = edit.ticket.revision;
    assert!(client
        .management_request(
            "POST",
            "/v1/tasks/submit",
            &serde_json::to_value(&submission).unwrap()
        )
        .await
        .is_err());
    submission.edit_token = Some(edit.edit_token);
    let saved: Ticket = serde_json::from_value(
        client
            .management_request(
                "POST",
                "/v1/tasks/submit",
                &serde_json::to_value(&submission).unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(!saved.draft);
}

#[tokio::test]
async fn client_observations_are_authenticated_distinct_and_ephemeral() {
    let srv = server().await;
    let http = Client::new();
    let id = RequestId::new().to_string();
    let path = "/v1/desktop/clients";
    // The observer itself has no client header and must not become a client node.
    let read = || request(&http, &srv, reqwest::Method::GET, path);
    assert!(read()
        .send()
        .await
        .unwrap()
        .json::<Vec<ClientPresence>>()
        .await
        .unwrap()
        .is_empty());
    let unauth = http
        .get(format!("{}{path}", srv.url))
        .header(VERSION_HEADER, "1")
        .header("x-factory-client-id", &id)
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);
    assert!(read()
        .send()
        .await
        .unwrap()
        .json::<Vec<ClientPresence>>()
        .await
        .unwrap()
        .is_empty());
    for (id, kind) in [(&id, "desktop"), (&RequestId::new().to_string(), "tui")] {
        assert!(read()
            .header("x-factory-client-id", id)
            .header("x-factory-client-kind", kind)
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
    }
    let observed = read()
        .send()
        .await
        .unwrap()
        .json::<Vec<ClientPresence>>()
        .await
        .unwrap();
    assert_eq!(observed.len(), 2);
    assert!(observed.iter().any(|c| c.name == "Desktop"));
    assert!(observed.iter().any(|c| c.name == "CLI / TUI"));
    assert!(observed.iter().all(|c| c.observed_unix_ms > 0));
    // Repeated calls from the same instance update rather than duplicate it.
    let body = read()
        .header("x-factory-client-id", &id)
        .header("x-factory-client-kind", "desktop")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!body.contains(TOKEN));
    assert_eq!(
        serde_json::from_str::<Vec<ClientPresence>>(&body)
            .unwrap()
            .len(),
        2
    );
    let other = server().await;
    assert!(request(&http, &other, reqwest::Method::GET, path)
        .send()
        .await
        .unwrap()
        .json::<Vec<ClientPresence>>()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn project_deletion_management_client_is_confirmed_and_replayed() {
    let server = server().await;
    let client = factory_protocol::client::FactoryClient::new(&server.url, TOKEN).unwrap();
    let p: Project = serde_json::from_value(
        client
            .management_request(
                "POST",
                "/v1/projects",
                &serde_json::to_value(CreateProject {
                    runner_mapping: None,
                    key: None,
                    request_id: RequestId::new(),
                    name: "Löschen bestätigen".into(),
                    repository: "factory:managed".into(),
                    base_branch: "main".into(),
                })
                .unwrap(),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let path = format!("/v1/projects/{}", p.id);
    let wrong = serde_json::to_value(DeleteProject {
        request_id: RequestId::new(),
        expected_revision: p.revision,
        confirm_name: "nicht bestätigt".into(),
    })
    .unwrap();
    assert!(client
        .management_request("DELETE", &path, &wrong)
        .await
        .is_err());
    let body = serde_json::to_value(DeleteProject {
        request_id: RequestId::new(),
        expected_revision: p.revision,
        confirm_name: p.name.clone(),
    })
    .unwrap();
    let receipt = client
        .management_request("DELETE", &path, &body)
        .await
        .unwrap();
    assert_eq!(receipt["deleted"], true);
    assert_eq!(
        client
            .management_request("DELETE", &path, &body)
            .await
            .unwrap(),
        receipt
    );
    let snapshot = client.snapshot().await.unwrap();
    assert!(snapshot.projects.is_empty());
}

#[tokio::test]
async fn command_poll_requires_current_runner_registration() {
    let server = server().await;
    let client = Client::new();
    let input = runner();
    let registration = request(
        &client,
        &server,
        reqwest::Method::POST,
        "/v1/runners/register",
    )
    .json(&input)
    .send()
    .await
    .unwrap()
    .json::<Registration>()
    .await
    .unwrap();
    let path = format!("/v1/runners/{}/commands", input.id);
    assert_eq!(
        request(&client, &server, reqwest::Method::GET, &path)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&client, &server, reqwest::Method::GET, &path)
            .header(RUNNER_HEADER, RegistrationId::new().to_string())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let response = request(&client, &server, reqwest::Method::GET, &path)
        .header(RUNNER_HEADER, registration.registration_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response
        .json::<Vec<RunnerCommand>>()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn container_reports_require_the_registered_freezer_capability() {
    let server = server().await;
    let client = Client::new();
    for capable in [false, true] {
        let mut input = runner();
        input.id = RunnerId::new();
        if capable {
            input.capabilities.push(CONTAINER_FREEZE_CAPABILITY.into());
        }
        let registration = request(
            &client,
            &server,
            reqwest::Method::POST,
            "/v1/runners/register",
        )
        .json(&input)
        .send()
        .await
        .unwrap()
        .json::<Registration>()
        .await
        .unwrap();
        let path = format!(
            "/v1/runners/{}/commands/{}/control",
            input.id,
            CommandId::new()
        );
        let response = request(&client, &server, reqwest::Method::POST, &path)
            .header(RUNNER_HEADER, registration.registration_id.to_string())
            .json(&ReportControl {
                proof: Some(ContainmentProof {
                    container_id: "a".repeat(64),
                    started_at: "2026-11-21T10:00:00Z".into(),
                    frozen: true,
                    observed_unix_ms: 1,
                }),
                control_request: RequestId::new(),
                ownership_epoch: 1,
                state: PauseState::Frozen,
                summary: "Frozen".into(),
            })
            .send()
            .await
            .unwrap();
        // Capability rejection precedes lookup; a capable runner reaches the missing command.
        assert_eq!(
            response.status(),
            if capable {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            }
        );
    }
}

#[tokio::test]
async fn project_ticket_routes_expose_revision_conflicts_and_durable_event_cursor() {
    let server = server().await;
    let client = Client::new();
    let project = request(&client, &server, reqwest::Method::POST, "/v1/projects")
        .json(&CreateProject {
            runner_mapping: None,
            key: None,
            request_id: RequestId::new(),
            name: "api-test".into(),
            repository: "/tmp/repo".into(),
            base_branch: "main".into(),
        })
        .send()
        .await
        .unwrap()
        .json::<Project>()
        .await
        .unwrap();
    let ticket = request(&client, &server, reqwest::Method::POST, "/v1/tickets")
        .json(&CreateTicket {
            request_id: RequestId::new(),
            project_id: project.id,
            title: "work".into(),
            description: "".into(),
            acceptance_criteria: "".into(),
            priority: 1,
        })
        .send()
        .await
        .unwrap()
        .json::<Ticket>()
        .await
        .unwrap();
    let path = format!("/v1/tickets/{}", ticket.id);
    let update = serde_json::json!({"request_id": RequestId::new(), "expected_revision": 1, "status":"ready"});
    let response = request(&client, &server, reqwest::Method::PATCH, &path)
        .json(&update)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let stale_id = RequestId::new();
    let response = request(&client, &server, reqwest::Method::PATCH, &path)
        .json(&serde_json::json!({"request_id": stale_id, "expected_revision": 1, "status":"done"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error = response.json::<ApiError>().await.unwrap();
    assert_eq!(error.code, ErrorCode::RevisionConflict);
    assert_eq!(error.current_revision, Some(Revision(2)));
    assert_eq!(error.request_id, stale_id);
    let snapshot = request(&client, &server, reqwest::Method::GET, "/v1/snapshot")
        .send()
        .await
        .unwrap()
        .json::<Snapshot>()
        .await
        .unwrap();
    assert_eq!(snapshot.event_cursor, 3);
    assert_eq!(snapshot.tickets[0].status, TicketState::Ready);
    let page = request(
        &client,
        &server,
        reqwest::Method::GET,
        "/v1/events?after=2&limit=1",
    )
    .send()
    .await
    .unwrap()
    .json::<EventPage>()
    .await
    .unwrap();
    assert_eq!(page.next_cursor, 3);
    assert_eq!(page.events[0].kind, "ticket.updated");
}

struct Server {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server() -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            factoryd::router(
                TOKEN,
                factoryd::store::Store::open(std::path::Path::new(":memory:")).unwrap(),
            ),
        )
        .await
        .unwrap();
    });
    Server { url, task }
}
fn runner() -> RegisterRunner {
    RegisterRunner {
        capabilities: vec![],
        id: RunnerId::new(),
        instance_id: InstanceId::new(),
        name: "laptop".into(),
        location: RunnerLocation::Local,
        version: "0.1.0".into(),
        opencode: OpenCodeStatus::NotConfigured,
    }
}
fn request(
    client: &Client,
    server: &Server,
    method: reqwest::Method,
    path: &str,
) -> reqwest::RequestBuilder {
    client
        .request(method, format!("{}{path}", server.url))
        .bearer_auth(TOKEN)
        .header(VERSION_HEADER, "1")
}

#[tokio::test]
async fn rejects_missing_auth_wrong_protocol_and_invalid_payload_without_leaking_body() {
    let server = server().await;
    let client = Client::new();
    assert_eq!(
        client
            .get(format!("{}/v1/snapshot", server.url))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mismatch = client
        .get(format!("{}/v1/snapshot", server.url))
        .bearer_auth(TOKEN)
        .header(VERSION_HEADER, "99")
        .send()
        .await
        .unwrap();
    assert_eq!(mismatch.status(), StatusCode::UPGRADE_REQUIRED);
    assert_eq!(
        mismatch.json::<ApiError>().await.unwrap().code,
        ErrorCode::ProtocolMismatch
    );
    let bad = request(
        &client,
        &server,
        reqwest::Method::POST,
        "/v1/runners/register",
    )
    .json(&serde_json::json!({"secret":"DO_NOT_ECHO"}))
    .send()
    .await
    .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    assert!(!bad.text().await.unwrap().contains("DO_NOT_ECHO"));
}

#[tokio::test]
async fn simultaneous_instances_cannot_steal_a_live_runner_and_stale_heartbeats_fail() {
    let server = server().await;
    let client = Client::new();
    let first = runner();
    let mut other = first.clone();
    other.instance_id = InstanceId::new();
    let a = request(
        &client,
        &server,
        reqwest::Method::POST,
        "/v1/runners/register",
    )
    .json(&first)
    .send();
    let b = request(
        &client,
        &server,
        reqwest::Method::POST,
        "/v1/runners/register",
    )
    .json(&other)
    .send();
    let (a, b) = tokio::join!(a, b);
    let a = a.unwrap();
    let b = b.unwrap();
    let (winner, response) = if a.status() == StatusCode::OK {
        assert_eq!(b.status(), StatusCode::CONFLICT);
        (&first, a)
    } else {
        assert_eq!(a.status(), StatusCode::CONFLICT);
        (&other, b)
    };
    let registration = response.json::<Registration>().await.unwrap();
    let retry = request(
        &client,
        &server,
        reqwest::Method::POST,
        "/v1/runners/register",
    )
    .json(winner)
    .send()
    .await
    .unwrap()
    .json::<Registration>()
    .await
    .unwrap();
    assert_eq!(registration.registration_id, retry.registration_id);
    let path = format!("/v1/runners/{}/heartbeat", first.id);
    let stale = request(&client, &server, reqwest::Method::POST, &path)
        .json(&Heartbeat {
            registration_id: RegistrationId::new(),
            opencode: OpenCodeStatus::Unavailable,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale.json::<ApiError>().await.unwrap().code,
        ErrorCode::RegistrationExpired
    );
    let ok = request(&client, &server, reqwest::Method::POST, &path)
        .json(&Heartbeat {
            registration_id: registration.registration_id,
            opencode: OpenCodeStatus::Ready {
                version: "2.0.27".into(),
            },
        })
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let snapshot = request(&client, &server, reqwest::Method::GET, "/v1/snapshot")
        .send()
        .await
        .unwrap()
        .json::<Snapshot>()
        .await
        .unwrap();
    assert_eq!(snapshot.runners.len(), 1);
    assert_eq!(snapshot.runners[0].connection, ConnectionState::Online);
    assert_eq!(
        snapshot.runners[0].opencode,
        OpenCodeStatus::Ready {
            version: "2.0.27".into()
        }
    );
}
