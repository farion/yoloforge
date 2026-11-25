use super::*;
use factory_protocol::mcp::{
    LocalMcpProbe, LocalMcpProbeStatus as ProbeStatus, LocalMcpProbeView,
    LOCAL_MCP_PROBE_CAPABILITY,
};
use serde::Deserialize;
use std::time::Duration;

const TTL: Duration = Duration::from_secs(45);
const RETAIN: Duration = Duration::from_secs(120);
pub(super) type Probes = BTreeMap<RequestId, Entry>;

pub(super) struct Entry {
    runner: RunnerId,
    registration: RegistrationId,
    operator: Option<DeviceId>,
    mcp: String,
    revision: Revision,
    claimed: bool,
    started: Instant,
    status: ProbeStatus,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Start {
    runner: RunnerId,
    expected_revision: Revision,
}

fn status_of(entry: &Entry) -> ProbeStatus {
    if entry.status == ProbeStatus::Pending && entry.started.elapsed() >= TTL {
        ProbeStatus::Unavailable
    } else {
        entry.status
    }
}

fn forbidden() -> Response {
    error(
        StatusCode::FORBIDDEN,
        ErrorCode::Unauthorized,
        "Nur Factory-Admins können lokale MCPs testen",
    )
}

fn unavailable() -> Response {
    error(
        StatusCode::CONFLICT,
        ErrorCode::InvalidRequest,
        "Der gespeicherte MCP oder der gewählte Runner ist nicht verfügbar",
    )
}

pub(super) async fn start(
    State(app): State<App>,
    Path(id): Path<String>,
    axum::Extension(principal): axum::Extension<Principal>,
    body: Result<Json<Start>, JsonRejection>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return forbidden();
    }
    let Ok(Json(body)) = body else {
        return unavailable();
    };
    let registration = app
        .registry
        .read()
        .await
        .entries
        .get(&body.runner)
        .filter(|entry| {
            entry.seen.elapsed() < SUSPECT_AFTER
                && matches!(entry.input.opencode, OpenCodeStatus::Ready { .. })
                && entry
                    .input
                    .capabilities
                    .iter()
                    .any(|c| c == LOCAL_MCP_PROBE_CAPABILITY)
        })
        .map(|entry| entry.registration);
    let Some(registration) = registration else {
        return unavailable();
    };
    let store = app.store.clone();
    let runner = body.runner;
    let mcp = id.clone();
    let revision = body.expected_revision;
    if !tokio::task::spawn_blocking(move || lookup(&store, runner, &mcp, revision).is_some())
        .await
        .unwrap_or(false)
    {
        return unavailable();
    }
    let mut entries = app.mcp_probes.lock().await;
    entries.retain(|_, entry| entry.started.elapsed() < RETAIN);
    if entries.len() >= 256
        || entries
            .values()
            .any(|entry| entry.runner == runner && status_of(entry) == ProbeStatus::Pending)
    {
        return unavailable();
    }
    let job = RequestId::new();
    entries.insert(
        job,
        Entry {
            runner,
            registration,
            operator: principal.device_id,
            mcp: id,
            revision,
            claimed: false,
            started: Instant::now(),
            status: ProbeStatus::Pending,
        },
    );
    Json(LocalMcpProbeView {
        id: job,
        status: ProbeStatus::Pending,
    })
    .into_response()
}

fn lookup(store: &Store, runner: RunnerId, mcp: &str, revision: Revision) -> Option<Vec<String>> {
    let catalog = store.local_mcp_catalog().ok()?;
    if catalog.revision != revision {
        return None;
    }
    catalog
        .entries
        .into_iter()
        .find(|entry| entry.id == mcp && entry.enabled_on(runner))
        .map(|entry| entry.command)
}

pub(super) async fn status(
    State(app): State<App>,
    Path(id): Path<RequestId>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return forbidden();
    }
    let entries = app.mcp_probes.lock().await;
    let Some(entry) = entries
        .get(&id)
        .filter(|entry| entry.operator == principal.device_id && entry.started.elapsed() < RETAIN)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    Json(LocalMcpProbeView {
        id,
        status: status_of(entry),
    })
    .into_response()
}

pub(super) async fn take(
    State(app): State<App>,
    Path(runner): Path<RunnerId>,
    headers: HeaderMap,
) -> Response {
    let guard = match runner_access(&app, runner, &headers).await {
        Ok(guard) => guard,
        Err(error) => return *error,
    };
    let registration = guard.entries[&runner].registration;
    let candidate = {
        let mut entries = app.mcp_probes.lock().await;
        entries
            .iter_mut()
            .find(|(_, entry)| {
                entry.runner == runner
                    && entry.registration == registration
                    && !entry.claimed
                    && status_of(entry) == ProbeStatus::Pending
            })
            .map(|(id, entry)| {
                entry.claimed = true;
                (*id, entry.mcp.clone(), entry.revision, entry.operator)
            })
    };
    drop(guard);
    let Some((id, mcp, revision, operator)) = candidate else {
        return Json(None::<LocalMcpProbe>).into_response();
    };
    let store = app.store.clone();
    let command = tokio::task::spawn_blocking(move || {
        let principal = Principal {
            role: DeviceRole::Admin,
            runner_id: None,
            device_id: operator,
        };
        if !principal.active(&store) {
            return None;
        }
        lookup(&store, runner, &mcp, revision)
    })
    .await
    .ok()
    .flatten();
    if command.is_none() {
        if let Some(entry) = app.mcp_probes.lock().await.get_mut(&id) {
            entry.status = ProbeStatus::Unavailable;
        }
    }
    Json(command.map(|command| LocalMcpProbe { id, command })).into_response()
}

pub(super) async fn finish(
    State(app): State<App>,
    Path(runner): Path<RunnerId>,
    headers: HeaderMap,
    body: Json<LocalMcpProbeView>,
) -> Response {
    let guard = match runner_access(&app, runner, &headers).await {
        Ok(guard) => guard,
        Err(error) => return *error,
    };
    let mut entries = app.mcp_probes.lock().await;
    let Some(entry) = entries.get_mut(&body.id).filter(|entry| {
        entry.runner == runner
            && entry.registration == guard.entries[&runner].registration
            && entry.claimed
            && status_of(entry) == ProbeStatus::Pending
            && body.status != ProbeStatus::Pending
    }) else {
        return unavailable();
    };
    entry.status = body.status;
    Json(LocalMcpProbeView {
        id: body.id,
        status: body.status,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_protocol::mcp::{LocalMcpCatalog, SetLocalMcpCatalog};
    use reqwest::Method;
    use serde_json::{json, Value};

    #[tokio::test]
    async fn only_the_assigned_registration_can_take_and_finish_a_probe_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db")).unwrap();
        let runner = RunnerId::new();
        let other = RunnerId::new();
        let program = std::env::current_exe().unwrap();
        let program = program.to_str().unwrap();
        let catalog = store
            .save_local_mcp_catalog(SetLocalMcpCatalog {
                request_id: RequestId::new(),
                catalog: LocalMcpCatalog {
                    revision: Revision(0),
                    entries: vec![LocalMcp {
                        all_runners: false, excluded_runners: vec![],
                        id: "blender".into(),
                        name: "Blender".into(),
                        runners: vec![runner],
                        command: vec![program.into()],
                    }],
                },
            })
            .unwrap();
        let app = router("fixture-admin-token", store.clone());
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", socket.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let ask = |method, path: &str| {
            client
                .request(method, format!("{url}{path}"))
                .bearer_auth("fixture-admin-token")
                .header(VERSION_HEADER, "1")
        };
        let register = |id| {
            ask(Method::POST, "/v1/runners/register").json(&RegisterRunner {
                id,
                instance_id: InstanceId::new(),
                name: "Runner".into(),
                location: RunnerLocation::Local,
                capabilities: vec![LOCAL_MCP_PROBE_CAPABILITY.into()],
                version: "fixture".into(),
                opencode: OpenCodeStatus::Ready {
                    version: "2.4.1".into(),
                },
            })
        };
        let a = register(runner)
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()["registration_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let b = register(other)
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()["registration_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let start = || {
            ask(Method::POST, "/v1/mcp/local/blender/test")
                .json(&json!({"runner":runner,"expected_revision":catalog.revision}))
        };
        let created = start()
            .send()
            .await
            .unwrap()
            .json::<LocalMcpProbeView>()
            .await
            .unwrap();
        assert_eq!(created.status, ProbeStatus::Pending);
        assert_eq!(start().send().await.unwrap().status(), StatusCode::CONFLICT);
        let take = |id, registration: &str| {
            ask(Method::POST, &format!("/v1/runners/{id}/mcp-tests/take"))
                .header(RUNNER_HEADER, registration)
        };
        assert_eq!(
            take(other, &b)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap(),
            Value::Null
        );
        let received = take(runner, &a)
            .send()
            .await
            .unwrap()
            .json::<LocalMcpProbe>()
            .await
            .unwrap();
        assert_eq!(received.id, created.id);
        assert_eq!(received.command, [program]);
        assert_eq!(
            take(runner, &a)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap(),
            Value::Null
        );
        let finish = |id, registration: &str| {
            ask(Method::POST, &format!("/v1/runners/{id}/mcp-tests"))
                .header(RUNNER_HEADER, registration)
                .json(&LocalMcpProbeView {
                    id: created.id,
                    status: ProbeStatus::Connected,
                })
        };
        assert_eq!(
            finish(other, &b).send().await.unwrap().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            finish(runner, &a).send().await.unwrap().status(),
            StatusCode::OK
        );
        let observed = ask(Method::GET, &format!("/v1/mcp/local-tests/{}", created.id))
            .send()
            .await
            .unwrap()
            .json::<LocalMcpProbeView>()
            .await
            .unwrap();
        assert_eq!(observed.status, ProbeStatus::Connected);
        let stale = start()
            .send()
            .await
            .unwrap()
            .json::<LocalMcpProbeView>()
            .await
            .unwrap();
        store
            .save_local_mcp_catalog(SetLocalMcpCatalog {
                request_id: RequestId::new(),
                catalog,
            })
            .unwrap();
        assert_eq!(
            take(runner, &a)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap(),
            Value::Null
        );
        let observed = ask(Method::GET, &format!("/v1/mcp/local-tests/{}", stale.id))
            .send()
            .await
            .unwrap()
            .json::<LocalMcpProbeView>()
            .await
            .unwrap();
        assert_eq!(observed.status, ProbeStatus::Unavailable);
        server.abort();
    }
}
