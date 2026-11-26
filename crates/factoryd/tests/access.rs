use factory_core::*;
use factory_protocol::*;
use factoryd::store::Store;
use reqwest::{Client, Method, StatusCode};
use std::time::Duration;
const ROOT: &str = "admin-test-token-0123456789abcdef";

#[tokio::test]
async fn admin_issues_scoped_keys_and_can_revoke_them_live() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = factoryd::router(ROOT, store.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = Client::new();
    let request = |method: Method, path: &str, token: &str| {
        client
            .request(method, format!("{url}{path}"))
            .bearer_auth(token)
            .header(VERSION_HEADER, "1")
    };
    let operator = CreateDevice {
        name: "Android".into(),
        role: DeviceRole::Operator,
        runner_id: None,
    };
    assert_eq!(
        request(Method::POST, "/v1/devices", "")
            .json(&operator)
            .send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let response = request(Method::POST, "/v1/devices", ROOT)
        .json(&operator)
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let credential = response.json::<DeviceCredential>().await.unwrap();
    for path in ["/v1/enroll", "/v1/devices/enrollment"] {
        assert_eq!(
            request(Method::POST, path, ROOT).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        request(Method::POST, "/v1/devices", &credential.token)
            .json(&operator).send().await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(Method::GET, "/v1/snapshot", &credential.token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        request(Method::GET, "/v1/devices", &credential.token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let mut events = request(Method::GET, "/v1/events/stream", &credential.token)
        .send()
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let first = tokio::time::timeout(Duration::from_secs(3), events.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&first).contains("device.created"));
    let revoke = format!("/v1/devices/{}/revoke", credential.device.id);
    assert_eq!(
        request(Method::POST, &revoke, ROOT)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while events.chunk().await.unwrap().is_some() {}
    })
    .await
    .unwrap();
    assert_eq!(
        request(Method::GET, "/v1/snapshot", &credential.token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let runner = RunnerId::new();
    let token = store
        .create_device(CreateDevice {
            name: "runner".into(),
            role: DeviceRole::Runner,
            runner_id: Some(runner),
        })
        .unwrap()
        .token;
    assert_eq!(
        request(Method::GET, "/v1/snapshot", &token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let body = RegisterRunner {
        capabilities: vec![],
        id: RunnerId::new(),
        instance_id: InstanceId::new(),
        name: "runner".into(),
        location: RunnerLocation::Local,
        version: "test".into(),
        opencode: OpenCodeStatus::NotConfigured,
    };
    assert_eq!(
        request(Method::POST, "/v1/runners/register", &token)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let body = RegisterRunner { id: runner, ..body };
    let registration = request(Method::POST, "/v1/runners/register", &token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json::<Registration>()
        .await
        .unwrap();
    assert_eq!(
        request(
            Method::GET,
            &format!("/v1/runners/{}/commands", RunnerId::new()),
            &token
        )
        .send()
        .await
        .unwrap()
        .status(),
        StatusCode::FORBIDDEN
    );
    let poll = request(
        Method::GET,
        &format!("/v1/runners/{runner}/commands?wait_seconds=10"),
        &token,
    )
    .header(RUNNER_HEADER, registration.registration_id.to_string())
    .send();
    let device = store
        .devices()
        .unwrap()
        .into_iter()
        .find(|d| d.runner_id == Some(runner))
        .unwrap();
    let stop = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        store.revoke_device(device.id).unwrap();
    };
    let (response, ()) = tokio::join!(poll, stop);
    assert_eq!(response.unwrap().status(), StatusCode::UNAUTHORIZED);
    server.abort();
}

#[test]
fn migration_removes_old_codes_without_invalidating_device_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let credential = store
        .create_device(CreateDevice {
            name: "admin".into(),
            role: DeviceRole::Admin,
            runner_id: None,
        })
        .unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TABLE enrollments (code_hash TEXT PRIMARY KEY, expires_unix_ms INTEGER, data TEXT);
         INSERT INTO enrollments VALUES ('old-code-hash', 9999999999999, '{}');
         PRAGMA user_version = 47;",
    ).unwrap();
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.authenticate_device(&credential.token).unwrap(),
        Some(credential.device)
    );
    let stored: String = db
        .query_row("SELECT token_hash || data FROM devices", [], |r| r.get(0))
        .unwrap();
    assert!(!stored.contains(&credential.token));
    assert_eq!(
        db.query_row("SELECT count(*) FROM sqlite_master WHERE name = 'enrollments'", [], |r| r.get::<_, u32>(0)).unwrap(),
        0,
    );
}
