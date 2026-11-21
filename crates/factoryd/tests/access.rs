use factory_core::*;
use factory_protocol::*;
use factoryd::store::Store;
use reqwest::{Client, Method, StatusCode};
use serde_json::json;
use std::time::Duration;
const ROOT: &str = "admin-test-token-0123456789abcdef";

#[tokio::test]
async fn device_roles_one_use_enrollment_and_live_revocation() {
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
    let grant = request(Method::POST, "/v1/devices/enrollment", ROOT)
        .json(&CreateEnrollment {
            role: DeviceRole::Operator,
            runner_id: None,
            ttl_seconds: 60,
        })
        .send()
        .await
        .unwrap()
        .json::<Enrollment>()
        .await
        .unwrap();
    let credential = request(Method::POST, "/v1/enroll", "")
        .json(&json!({"code":grant.code,"name":"laptop"}))
        .send()
        .await
        .unwrap()
        .json::<DeviceCredential>()
        .await
        .unwrap();
    assert_eq!(
        request(Method::POST, "/v1/enroll", "")
            .json(&json!({"code":grant.code,"name":"replay"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
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
    assert!(String::from_utf8_lossy(&first).contains("device.enrolled"));
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
    let grant = store
        .create_enrollment(CreateEnrollment {
            role: DeviceRole::Runner,
            runner_id: Some(runner),
            ttl_seconds: 60,
        })
        .unwrap();
    let token = store
        .redeem_enrollment(RedeemEnrollment {
            code: grant.code,
            name: "runner".into(),
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
fn device_secrets_are_hashed_and_survive_restart_codes_expire() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let grant = store
        .create_enrollment(CreateEnrollment {
            role: DeviceRole::Admin,
            runner_id: None,
            ttl_seconds: 60,
        })
        .unwrap();
    let credential = store
        .redeem_enrollment(RedeemEnrollment {
            code: grant.code,
            name: "admin".into(),
        })
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.authenticate_device(&credential.token).unwrap(),
        Some(credential.device)
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    let stored: String = db
        .query_row("SELECT token_hash || data FROM devices", [], |r| r.get(0))
        .unwrap();
    assert!(!stored.contains(&credential.token));
    let grant = store
        .create_enrollment(CreateEnrollment {
            role: DeviceRole::Operator,
            runner_id: None,
            ttl_seconds: 30,
        })
        .unwrap();
    db.execute("UPDATE enrollments SET expires_unix_ms=0", [])
        .unwrap();
    assert!(store
        .redeem_enrollment(RedeemEnrollment {
            code: grant.code,
            name: "expired".into()
        })
        .is_err());
}
