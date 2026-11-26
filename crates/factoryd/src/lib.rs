use axum::{
    extract::{rejection::JsonRejection, Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use factory_core::*;
use factory_protocol::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;
pub mod push;
pub mod store;
use store::{Store, StoreError};
mod access;
mod archive_routes;
mod checkpoint_routes;
mod compaction_routes;
mod direct_routes;
mod desktop;
mod knowledge_routes;
mod live;
mod mcp_probe;
mod mcp_routes;
pub mod model_gateway;
mod model_routes;
pub mod remote_mcp;
mod review_routes;
mod scheduler_routes;
mod skill_routes;
mod supervisor_routes;
mod termination_routes;
mod work_routes;
use access::*;
use live::*;
use work_routes::*;

struct Entry {
    ip: Option<std::net::IpAddr>,
    input: RegisterRunner,
    registration: RegistrationId,
    seen: Instant,
    unix_ms: u64,
}
#[derive(Default)]
struct Registry {
    clients: BTreeMap<String, ClientPresence>,
    instances: BTreeMap<RunnerId, Vec<OpenCodeInstance>>,
    entries: BTreeMap<RunnerId, Entry>,
    revision: Revision,
}
#[derive(Clone)]
struct App {
    registry: Arc<RwLock<Registry>>,
    token_hash: Option<[u8; 32]>,
    boot_id: BootId,
    store: Store,
    stopping: Arc<std::sync::atomic::AtomicBool>,
    mcp_probes: Arc<tokio::sync::Mutex<mcp_probe::Probes>>,
}

pub fn router(token: &str, store: Store) -> Router {
    router_with_shutdown(
        token,
        store,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
}
pub fn router_with_shutdown(
    token: &str,
    store: Store,
    stopping: Arc<std::sync::atomic::AtomicBool>,
) -> Router {
    let state = App {
        registry: Arc::default(),
        token_hash: (!token.is_empty()).then(|| Sha256::digest(token.as_bytes()).into()),
        boot_id: BootId::new(),
        store,
        stopping,
        mcp_probes: Default::default(),
    };
    Router::new()
        .route("/v1/instances", get(desktop::instances))
        .route("/v1/me", get(desktop::me))
        .route("/v1/desktop/snapshot", get(desktop_snapshot))
        .route("/v1/desktop/clients", get(desktop::clients))
        .route("/v1/desktop/events", get(desktop::desktop_events))
        .route("/v1/desktop/events/stream", get(live::desktop_event_stream))
        .route("/v1/events/{id}", get(desktop::event_payload))
        .route("/v1/tasks/{id}", get(desktop::task_payload).delete(desktop::delete_failed_task))
        .route("/v1/tasks/{id}/inactive", axum::routing::delete(desktop::delete_inactive_task))
        .route("/v1/tasks/{id}/intent", axum::routing::delete(desktop::delete_task))
        .route("/v1/tasks/{id}/history", get(desktop::task_history))
        .route("/v1/tasks/{id}/models", get(model_routes::task))
        .route(
            "/v1/model-settings",
            get(model_routes::factory).put(model_routes::set_factory),
        )
        .route("/v1/model-settings/setup", get(model_routes::setup))
        .route(
            "/v1/model-settings/accesses/{access}",
            axum::routing::delete(model_gateway::remove_access),
        )
        .route(
            "/v1/model-settings/accesses/{access}/status",
            get(model_gateway::status),
        )
        .route(
            "/v1/model-settings/accesses/{access}/key",
            post(model_gateway::key_provider::connect),
        )
        .route(
            "/v1/model-settings/opencode-free",
            post(model_gateway::opencode_free::connect),
        )
        .route(
            "/v1/model-settings/accesses/{access}/{provider}/oauth",
            post(model_gateway::begin),
        )
        .route(
            "/v1/model-settings/accesses/{access}/{provider}/oauth/start",
            post(model_gateway::begin_named),
        )
        .route(
            "/v1/model-settings/accesses/{access}/{provider}/oauth/complete",
            post(model_gateway::finish),
        )
        .route(
            "/v1/model-settings/accesses/{access}/{provider}/oauth/cancel",
            post(model_gateway::cancel),
        )
        .route(
            "/v1/projects/{id}/models",
            get(model_routes::project).put(model_routes::set_project),
        )
        .route(
            "/v1/projects/{id}/skills",
            get(skill_routes::read).put(skill_routes::save),
        )
        .route(
            "/v1/skills",
            get(skill_routes::catalog).post(skill_routes::import),
        )
        .route(
            "/v1/runners/{runner}/commands/{command}/skills/{id}/{version}",
            get(skill_routes::bundle),
        )
        .route(
            "/v1/mcp/local",
            get(mcp_routes::catalog).put(mcp_routes::save_catalog),
        )
        .route("/v1/mcp/local/choices", get(mcp_routes::choices))
        .route("/v1/mcp/local/{id}/test", post(mcp_probe::start))
        .route("/v1/mcp/local-tests/{id}", get(mcp_probe::status))
        .route("/v1/runners/{runner}/mcp-tests/take", post(mcp_probe::take))
        .route("/v1/runners/{runner}/mcp-tests", post(mcp_probe::finish))
        .route("/v1/mcp/remote", get(remote_mcp::list))
        .route(
            "/v1/mcp/remote/discover",
            post(remote_mcp::discovery::discover),
        )
        .route(
            "/mcp-oauth/client.json",
            get(remote_mcp::discovery::client_metadata),
        )
        .route(
            "/v1/mcp/remote/{id}",
            axum::routing::put(remote_mcp::save).delete(remote_mcp::remove),
        )
        .route(
            "/v1/mcp/remote/{id}/test",
            post(remote_mcp::test_connection),
        )
        .route(
            "/v1/mcp/remote/{id}/oauth/start",
            post(remote_mcp::oauth::start),
        )
        .route(
            "/v1/mcp/remote/{id}/oauth/prepare",
            post(remote_mcp::oauth::prepare),
        )
        .route(
            "/v1/mcp/remote/{id}/oauth/commit",
            post(remote_mcp::oauth::commit),
        )
        .route("/v1/mcp/remote/{id}/verify", post(remote_mcp::verify_draft))
        .route(
            "/v1/mcp/remote/{id}/oauth/status",
            post(remote_mcp::oauth::status),
        )
        .route(
            "/v1/mcp/remote/{id}/oauth/complete",
            post(remote_mcp::oauth::complete_loopback),
        )
        .route("/mcp-oauth/callback", get(remote_mcp::oauth::callback))
        .route(
            "/v1/runners/{runner}/commands/{command}/mcp/{id}",
            get(remote_mcp::lease),
        )
        .route(
            "/mcp-runtime/{runner}/{command}/{id}",
            get(remote_mcp::relay)
                .post(remote_mcp::relay)
                .delete(remote_mcp::relay),
        )
        .route(
            "/v1/projects/{id}/mcp/local",
            get(mcp_routes::project).put(mcp_routes::save_project),
        )
        .route("/v1/tasks/{id}/edit", post(desktop::begin_task_edit))
        .route("/v1/tasks/{id}/retry", post(desktop::retry_task))
        .route("/v1/tasks/{id}/accept-solution", post(desktop::accept_intent_solution))
        .route("/v1/tasks/{id}/return-to-draft", post(desktop::return_intent_to_draft))
        .route("/v1/runners/{id}/restart",
            get(desktop::runner_restart_status).put(desktop::hold_runner_restart).delete(desktop::release_runner_restart))
        .route("/v1/commands/{id}", get(desktop::command_payload))
        .route("/v1/events/history", get(desktop::event_history))
        .route(
            "/v1/commands/{id}/archive/window",
            get(desktop::archive_window),
        )
        .route(
            "/v1/commands/{id}/runtime-binding",
            get(desktop::runtime_binding),
        )
        .route(
            "/v1/runners/{runner}/instances",
            axum::routing::put(desktop::report_instances),
        )
        .route("/v1/notifications", get(desktop::notifications))
        .route("/v1/notifications/read", post(desktop::mark_notifications))
        .route("/v1/projects/{id}/directs", get(direct_routes::list))
        .route("/v1/directs", post(direct_routes::create))
        .route("/v1/directs/{id}/prompt", post(direct_routes::prompt))
        .route("/v1/directs/{id}/archive", post(direct_routes::archive))
        .route(
            "/v1/notifications/status",
            post(desktop::notification_states),
        )
        .route(
            "/v1/projects/{id}/supervisor",
            get(supervisor_routes::read).put(supervisor_routes::configure),
        )
        .route(
            "/v1/projects/{id}/supervisor/evaluations",
            get(supervisor_routes::evaluations),
        )
        .route(
            "/v1/commands/{id}/closure",
            get(compaction_routes::closure).post(compaction_routes::close_session),
        )
        .route(
            "/v1/streams/{id}/continue",
            post(compaction_routes::continue_agent),
        )
        .route(
            "/v1/commands/{id}/compaction",
            get(compaction_routes::read).post(compaction_routes::create),
        )
        .route(
            "/v1/runners/{runner}/commands/{id}/compaction",
            get(compaction_routes::runner_read).put(compaction_routes::report),
        )
        .route("/v1/commands/{id}/archive", get(archive_routes::read))
        .route("/v1/commands/{id}/files", get(archive_routes::files))
        .route(
            "/v1/commands/{id}/files/{hash}/{offset}",
            get(archive_routes::file_chunk),
        )
        .route(
            "/v1/runners/{runner}/commands/{id}/files",
            axum::routing::put(archive_routes::write_file)
                .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024)),
        )
        .route(
            "/v1/runners/{runner}/commands/{id}/archive",
            axum::routing::put(archive_routes::write)
                .get(archive_routes::runner_read)
                .layer(axum::extract::DefaultBodyLimit::max(9 * 1024 * 1024)),
        )
        .route("/v1/knowledge", post(knowledge_routes::create))
        .route("/v1/knowledge/{id}", get(knowledge_routes::read))
        .route("/v1/knowledge/{id}/retire", post(knowledge_routes::retire))
        .route("/v1/projects/{id}/knowledge", get(knowledge_routes::list))
        .route("/v1/projects/{id}/usage", get(knowledge_routes::usage))
        .route(
            "/v1/projects/{id}/token-budget",
            get(knowledge_routes::budget).put(knowledge_routes::set_budget),
        )
        .route("/v1/projects/{id}/context", get(knowledge_routes::context))
        .route("/healthz", get(health))
        .route(
            "/v1/enroll",
            post(redeem).layer(axum::extract::DefaultBodyLimit::max(4096)),
        )
        .route("/v1/devices", get(list_devices))
        .route("/v1/devices/enrollment", post(enrollment))
        .route("/v1/devices/{id}/revoke", post(revoke))
        .route("/v1/snapshot", get(snapshot))
        .route("/v1/projects", post(create_project))
        .route("/v1/commands/{id}/excerpt", get(command_excerpt))
        .route(
            "/v1/runners/{runner}/commands/{command}/excerpt",
            axum::routing::put(report_excerpt),
        )
        .route(
            "/v1/projects/{id}",
            patch(update_project).delete(review_routes::delete_project),
        )
        .route("/v1/projects/{id}/mirror", axum::routing::put(set_project_mirror))
        .route("/v1/questions/{id}/answer", post(answer_question))
        .route(
            "/v1/runners/{runner}/commands/{command}/questions",
            post(sync_questions),
        )
        .route(
            "/v1/runners/{runner}/questions/{id}/delivery",
            post(answer_delivery),
        )
        .route("/v1/projects/{id}/profile", patch(update_profile))
        .route("/v1/commands/{id}/hosting", post(review_routes::hosted))
        .route("/v1/tickets", post(create_ticket))
        .route("/v1/tickets/{id}", patch(update_ticket))
        .route("/v1/tickets/{id}/reconcile-publication", post(reconcile_publication))
        .route("/v1/streams", post(create_stream))
        .route("/v1/streams/{id}/control", post(set_control))
        .route("/v1/projects/setup", post(review_routes::setup))
        .route(
            "/v1/projects/{id}/session-limit",
            post(review_routes::session_limit),
        )
        .route(
            "/v1/projects/{id}/pause",
            post(review_routes::pause_project),
        )
        .route(
            "/v1/tasks",
            post(review_routes::task).layer(axum::extract::DefaultBodyLimit::max(6 * 1024 * 1024)),
        )
        .route(
            "/v1/tasks/submit",
            post(review_routes::submit_task)
                .layer(axum::extract::DefaultBodyLimit::max(12 * 1024 * 1024)),
        )
        .route(
            "/v1/tasks/submit-with-model",
            post(model_routes::submit)
                .layer(axum::extract::DefaultBodyLimit::max(12 * 1024 * 1024)),
        )
        .route(
            "/v1/projects/{id}/scheduler",
            get(scheduler_routes::read).post(scheduler_routes::configure),
        )
        .route(
            "/v1/tickets/{id}/dependencies",
            post(scheduler_routes::dependencies),
        )
        .route("/v1/streams/{id}/review", post(review_routes::review))
        .route("/v1/streams/{id}/integrate", post(review_routes::integrate))
        .route("/v1/commands/{id}/review", get(review_routes::read))
        .route(
            "/v1/runners/{runner}/commands/{command}/review",
            get(review_routes::runner_read),
        )
        .route(
            "/v1/runners/{runner}/runtime-cleanups",
            get(termination_routes::pending),
        )
        .route("/v1/streams/{id}/handoff", post(checkpoint_routes::handoff))
        .route(
            "/v1/runners/{runner}/commands/{command}/checkpoint-bundle",
            get(checkpoint_routes::download)
                .post(checkpoint_routes::upload)
                .layer(axum::extract::DefaultBodyLimit::max(
                    CHECKPOINT_BUNDLE_LIMIT,
                )),
        )
        .route("/v1/commands/{id}/checkpoint", get(checkpoint_routes::read))
        .route(
            "/v1/runners/{runner}/commands/{command}/checkpoint",
            get(checkpoint_routes::runner_read),
        )
        .route(
            "/v1/runners/{runner}/commands/{command}/runtime-binding",
            post(termination_routes::bind),
        )
        .route(
            "/v1/commands/{id}/termination",
            get(termination_routes::read).post(termination_routes::create),
        )
        .route(
            "/v1/runners/{runner}/commands/{command}/termination",
            get(termination_routes::runner_read).post(termination_routes::report),
        )
        .route("/v1/streams/{id}/control/retry", post(retry_control))
        .route(
            "/v1/runners/{runner}/commands/{command}/control",
            get(runner_control).post(report_control),
        )
        .route(
            "/v1/runners/{runner}/commands/{command}/observe",
            post(observe_agent),
        )
        .route("/v1/streams/{id}/commands", post(enqueue))
        .route("/v1/runners/{id}/commands", get(pending_commands))
        .route("/v1/runners/{id}/draft-reset", get(work_routes::pending_draft_reset))
        .route("/v1/runners/{runner}/draft-reset/{request}", post(work_routes::confirm_draft_reset))
        .route(
            "/v1/runners/{runner}/commands/{command}/ack",
            post(ack_command),
        )
        .route(
            "/v1/runners/{runner}/commands/{command}/result",
            post(command_result),
        )
        .route(
            "/v1/runners/{runner}/commands/{command}/publication-recovery",
            axum::routing::put(work_routes::publication_recovery)
                .post(work_routes::report_recovered_publication)
                .delete(work_routes::release_publication_recovery),
        )
        .route("/v1/events", get(events))
        .route("/v1/events/stream", get(event_stream))
        .route("/v1/runners/register", post(register))
        .route("/v1/runners/{id}/heartbeat", post(heartbeat))
        .route("/v1/runners/{id}/snapshot", get(runner_snapshot))
        .route(
            "/v1/commands/{root}/prompt",
            post(review_routes::prompt_session)
                .layer(axum::extract::DefaultBodyLimit::max(12 * 1024 * 1024)),
        )
        .fallback(|| async {
            error(
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
                "Route not found",
            )
        })
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state)
}

pub fn router_with_model_gateway(
    token: &str,
    store: Store,
    stopping: Arc<std::sync::atomic::AtomicBool>,
    gateway: Option<model_gateway::Gateway>,
) -> Router {
    let router = router_with_shutdown(token, store.clone(), stopping);
    match gateway {
        Some(gateway) => router
            .layer(axum::Extension(gateway.clone()))
            .merge(model_gateway::routes(gateway, store)),
        None => router,
    }
}

fn error(status: StatusCode, code: ErrorCode, message: &str) -> Response {
    (
        status,
        Json(ApiError {
            code,
            message: message.into(),
            request_id: RequestId::new(),
            current_revision: None,
        }),
    )
        .into_response()
}
async fn authenticate(State(app): State<App>, mut request: Request, next: Next) -> Response {
    if matches!(
        request.uri().path(),
        "/healthz" | "/mcp-oauth/callback" | "/mcp-oauth/client.json"
    ) || request.uri().path().starts_with("/mcp-runtime/")
    {
        return next.run(request).await;
    }
    if request.uri().path() == "/v1/enroll" {
        if request
            .headers()
            .get(VERSION_HEADER)
            .and_then(|v| v.to_str().ok())
            != Some("1")
        {
            return error(
                StatusCode::UPGRADE_REQUIRED,
                ErrorCode::ProtocolMismatch,
                "Factory protocol 1 required",
            );
        }
        return next.run(request).await;
    }
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let principal = if app
        .token_hash
        .as_ref()
        .is_some_and(|expected| bool::from(hash.ct_eq(expected)))
    {
        Principal {
            role: DeviceRole::Admin,
            runner_id: None,
            device_id: None,
        }
    } else {
        if token.len() > 100 {
            return error(
                StatusCode::UNAUTHORIZED,
                ErrorCode::Unauthorized,
                "Factory credential required",
            );
        }
        let store = app.store.clone();
        let token = token.to_string();
        match tokio::task::spawn_blocking(move || store.authenticate_device(&token)).await {
            Ok(Ok(Some(device))) => Principal {
                role: device.role,
                runner_id: device.runner_id,
                device_id: Some(device.id),
            },
            Ok(Ok(None)) => {
                return error(
                    StatusCode::UNAUTHORIZED,
                    ErrorCode::Unauthorized,
                    "Factory credential required",
                )
            }
            _ => {
                return error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorCode::Internal,
                    "Credential store unavailable",
                )
            }
        }
    };
    if request
        .headers()
        .get(VERSION_HEADER)
        .and_then(|v| v.to_str().ok())
        != Some("1")
    {
        return error(
            StatusCode::UPGRADE_REQUIRED,
            ErrorCode::ProtocolMismatch,
            "Factory protocol 1 required",
        );
    }
    if !principal.allows(request.uri().path()) {
        return error(
            StatusCode::FORBIDDEN,
            ErrorCode::Unauthorized,
            "Device role does not permit this route",
        );
    }
    let ip = observed_ip(&request);
    desktop::observe_client(&app, &principal, request.headers(), ip).await;
    request.extensions_mut().insert(ObservedIp(ip));
    request.extensions_mut().insert(principal);
    next.run(request).await
}
fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn registration(app: &App, id: RegistrationId) -> Json<Registration> {
    Json(Registration {
        registration_id: id,
        boot_id: app.boot_id,
        heartbeat_seconds: HEARTBEAT_INTERVAL.as_secs(),
    })
}
fn valid_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
#[derive(Clone, Copy)]
struct ObservedIp(Option<std::net::IpAddr>);

fn observed_ip(request: &Request) -> Option<std::net::IpAddr> {
    request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|peer| peer.0.ip().to_canonical())
}

fn local_project_source(
    principal: &Principal, runner: RunnerId, location: RunnerLocation,
    capable: bool, peer: ObservedIp,
) -> bool {
    // Native desktop sources are reached over HTTPS too. A device
    // credential bound to this runner is stronger than proxy topology.
    capable && location == RunnerLocation::Local
        && ((principal.role == DeviceRole::Runner && principal.runner_id == Some(runner))
            || peer.0.is_some_and(|ip| ip.is_loopback()))
}

#[cfg(test)]
#[test]
fn enrolled_desktop_can_supply_a_project_checkpoint_over_https() {
    let runner = RunnerId::new();
    let principal = Principal { role: DeviceRole::Runner, runner_id: Some(runner), device_id: None };
    let remote = ObservedIp(Some("192.0.2.10".parse().unwrap()));
    assert!(local_project_source(&principal, runner, RunnerLocation::Local, true, remote));
    assert!(!local_project_source(&principal, RunnerId::new(), RunnerLocation::Local, true, remote));
    assert!(!local_project_source(&principal, runner, RunnerLocation::Server, true, remote));
    assert!(!local_project_source(&principal, runner, RunnerLocation::Local, false, remote));
}

async fn register(
    State(app): State<App>,
    axum::Extension(principal): axum::Extension<Principal>,
    axum::Extension(peer): axum::Extension<ObservedIp>,
    headers: HeaderMap,
    body: Result<Json<RegisterRunner>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid registration JSON",
        );
    };
    if principal.runner_id.is_some_and(|id| id != body.id) {
        return error(
            StatusCode::FORBIDDEN,
            ErrorCode::Unauthorized,
            "Runner identity does not match device",
        );
    }
    if !valid_text(&body.name, 80)
        || !valid_text(&body.version, 80)
        || body.capabilities.len() > 22
        || body.capabilities.iter().any(|c| !valid_text(c, 80))
    {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid runner name or version",
        );
    }
    let mut registry = app.registry.write().await;
    let now = Instant::now();
    let mut id = RegistrationId::new();
    if let Some(existing) = registry.entries.get(&body.id) {
        if existing.input.instance_id == body.instance_id {
            id = existing.registration;
        } else if ConnectionState::from_age(now.duration_since(existing.seen))
            != ConnectionState::Unreachable
        {
            return error(
                StatusCode::CONFLICT,
                ErrorCode::RunnerConflict,
                "Runner ID already used by another live instance",
            );
        }
    }
    if body
        .capabilities
        .iter()
        .any(|c| c == TASK_IMAGES_CAPABILITY)
    {
        let store = app.store.clone();
        let runner = body.id;
        let local = body
            .capabilities
            .iter()
            .any(|c| c == LOCAL_PROJECT_CAPABILITY);
        let remote = body
            .capabilities
            .iter()
            .any(|c| c == mcp::REMOTE_MCP_CAPABILITY);
        let mcp_ready = (remote
            || body
                .capabilities
                .iter()
                .any(|c| c == mcp::LOCAL_MCP_CAPABILITY))
            && matches!(body.opencode, OpenCodeStatus::Ready { .. });
        let ready = matches!(body.opencode, OpenCodeStatus::Ready { .. });
        let local_source = local_project_source(&principal, runner, body.location, local, peer);
        let checkpoint = body.capabilities.iter().any(|c| c == AUTOMATIC_CHECKPOINT_CAPABILITY);
        let server_runner = body.location == RunnerLocation::Server;
        let native_boot = (checkpoint && body.location == RunnerLocation::Local
            && principal.role == DeviceRole::Runner && principal.runner_id == Some(runner))
            .then(|| headers.get(NATIVE_RUNNER_BOOT_HEADER)?.to_str().ok().and_then(NativeRunnerBoot::from_header))
            .flatten();
        if !matches!(
            tokio::task::spawn_blocking(move || {
                store.autostart_runner_at(runner, server_runner)?;
                store.observe_native_runner_boot(runner, native_boot)?;
                store.observe_mcp_runner(runner, mcp_ready, remote)?;
                store.observe_automatic_runner_location(runner, ready, local_source, checkpoint, server_runner)?;
                if local {
                    store.recover_local_projects(runner)?;
                }
                Ok::<_, StoreError>(())
            })
            .await,
            Ok(Ok(()))
        ) {
            return db_error(StoreError::Internal, RequestId::new());
        }
    }
    registry.entries.insert(
        body.id,
        Entry {
            ip: peer.0,
            input: body,
            registration: id,
            seen: now,
            unix_ms: unix_ms(),
        },
    );
    registry.revision.0 += 1;
    registration(&app, id).into_response()
}
async fn heartbeat(
    State(app): State<App>,
    axum::Extension(principal): axum::Extension<Principal>,
    axum::Extension(peer): axum::Extension<ObservedIp>,
    Path(id): Path<String>,
    body: Result<Json<Heartbeat>, JsonRejection>,
) -> Response {
    let Ok(id) = id.parse::<RunnerId>() else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid runner ID",
        );
    };
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid heartbeat JSON",
        );
    };
    let mut registry = app.registry.write().await;
    let Some(entry) = registry.entries.get_mut(&id) else {
        return error(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "Runner must register",
        );
    };
    if entry.registration != body.registration_id {
        return error(
            StatusCode::CONFLICT,
            ErrorCode::RegistrationExpired,
            "Runner registration no longer current",
        );
    }
    entry.seen = Instant::now();
    entry.unix_ms = unix_ms();
    entry.ip = peer.0;
    entry.input.opencode = body.opencode;
    if entry
        .input
        .capabilities
        .iter()
        .any(|c| c == TASK_IMAGES_CAPABILITY)
    {
        let store = app.store.clone();
        let local = entry
            .input
            .capabilities
            .iter()
            .any(|c| c == LOCAL_PROJECT_CAPABILITY);
        let remote = entry
            .input
            .capabilities
            .iter()
            .any(|c| c == mcp::REMOTE_MCP_CAPABILITY);
        let mcp_ready = (remote
            || entry
                .input
                .capabilities
                .iter()
                .any(|c| c == mcp::LOCAL_MCP_CAPABILITY))
            && matches!(entry.input.opencode, OpenCodeStatus::Ready { .. });
        let ready = matches!(entry.input.opencode, OpenCodeStatus::Ready { .. });
        let local_source = local_project_source(&principal, id, entry.input.location, local, peer);
        let checkpoint = entry.input.capabilities.iter().any(|c| c == AUTOMATIC_CHECKPOINT_CAPABILITY);
        let server_runner = entry.input.location == RunnerLocation::Server;
        if !matches!(
            tokio::task::spawn_blocking(move || {
                store.autostart_runner_at(id, server_runner)?;
                store.observe_mcp_runner(id, mcp_ready, remote)?;
                store.observe_automatic_runner_location(id, ready, local_source, checkpoint, server_runner)?;
                if local {
                    store.recover_local_projects(id)?;
                }
                Ok::<_, StoreError>(())
            })
            .await,
            Ok(Ok(()))
        ) {
            return db_error(StoreError::Internal, RequestId::new());
        }
    }
    registry.revision.0 += 1;
    registration(&app, body.registration_id).into_response()
}
async fn snapshot(State(app): State<App>) -> Response {
    snapshot_response(app, false).await
}

async fn runner_snapshot(State(app): State<App>, Path(id): Path<RunnerId>) -> Response {
    let boot = app.boot_id;
    db_call(RequestId::new(), move || {
        let mut snapshot = app.store.snapshot()?;
        snapshot.commands.retain(|c| c.runner_id == id);
        snapshot.streams.retain(|s| s.runner_id == id);
        snapshot.projects.retain(|p| snapshot.streams.iter().any(|s| s.project_id == p.id)
            || p.runner_bindings.contains_key(&id) || p.setup.as_ref().is_some_and(|setup| setup.runner_id == id));
        Ok(serde_json::json!({
            "protocol_version": PROTOCOL_VERSION, "server_version": env!("CARGO_PKG_VERSION"),
            "server_capabilities": [TARGET_GIT_CAPABILITY, RUNNER_RESTART_CAPABILITY, factory_protocol::directs::CAPABILITY, DELETE_FAILED_TASK_CAPABILITY, DELETE_INACTIVE_TASK_CAPABILITY, DELETE_TASK_CAPABILITY],
            "boot_id": boot, "revision": 0, "runners": [],
            "projects": snapshot.projects, "streams": snapshot.streams, "commands": snapshot.commands,
        }))
    }).await
}
async fn desktop_snapshot(State(app): State<App>) -> Response {
    snapshot_response(app, true).await
}
async fn snapshot_response(app: App, compact: bool) -> Response {
    let store = app.store.clone();
    let durable = match tokio::task::spawn_blocking(move || {
        let mut snapshot = if compact {
            store.desktop_snapshot()?
        } else {
            store.snapshot()?
        };
        if compact {
            for ticket in &mut snapshot.tickets {
                for image in &mut ticket.images {
                    image.data = String::new();
                }
                for file in &mut ticket.files {
                    file.data = String::new();
                }
            }
            for command in &mut snapshot.commands {
                if let Some(spec) = &mut command.agent_spec {
                    for image in &mut spec.images {
                        image.data = String::new();
                    }
                    for file in &mut spec.files {
                        file.data = String::new();
                    }
                }
            }
        }
        Ok::<_, StoreError>(snapshot)
    })
    .await
    {
        Ok(Ok(value)) => value,
        _ => return db_error(StoreError::Internal, RequestId::new()),
    };
    let registry = app.registry.read().await;
    Json(Snapshot {
        draft_resets: durable.draft_resets,
        intent_problems: durable.intent_problems,
        directs: durable.directs,
        server_capabilities: vec![TARGET_GIT_CAPABILITY.into(), RUNNER_RESTART_CAPABILITY.into(), factory_protocol::directs::CAPABILITY.into(), DELETE_FAILED_TASK_CAPABILITY.into(), DELETE_INACTIVE_TASK_CAPABILITY.into(), DELETE_TASK_CAPABILITY.into(), DRAFT_RESET_CAPABILITY.into()],
        token_totals: durable.token_totals,
        session_activity_unix_ms: durable.session_activity_unix_ms,
        editable_tasks: durable.editable_tasks,
        session_titles: durable.session_titles,
        payloads_deferred: compact,
        session_closures: durable.session_closures,
        compactions: durable.compactions,
        runtime_terminations: durable.runtime_terminations,
        scheduler_policies: durable.scheduler_policies,
        token_budgets: durable.token_budgets,
        protocol_version: PROTOCOL_VERSION,
        server_version: env!("CARGO_PKG_VERSION").into(),
        boot_id: app.boot_id,
        revision: registry.revision,
        runners: registry
            .entries
            .iter()
            .map(|(id, e)| RunnerView {
                ip: e.ip,
                capabilities: e.input.capabilities.clone(),
                id: *id,
                name: e.input.name.clone(),
                location: e.input.location,
                version: e.input.version.clone(),
                connection: ConnectionState::from_age(e.seen.elapsed()),
                last_seen_unix_ms: e.unix_ms,
                opencode: e.input.opencode.clone(),
            })
            .collect(),
        projects: durable.projects,
        tickets: durable.tickets,
        event_cursor: durable.cursor,
        streams: durable.streams,
        commands: durable.commands,
        questions: durable.questions,
        controls: durable.controls,
    })
    .into_response()
}

fn db_error(error: StoreError, request_id: RequestId) -> Response {
    let (status, code, message, revision) = match error {
        StoreError::Invalid(message) => (
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            message,
            None,
        ),
        StoreError::NotFound => (
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "Requested aggregate not found",
            None,
        ),
        StoreError::RequestConflict => (
            StatusCode::CONFLICT,
            ErrorCode::RequestConflict,
            "Request ID already used with different input",
            None,
        ),
        StoreError::RevisionConflict(rev) => (
            StatusCode::CONFLICT,
            ErrorCode::RevisionConflict,
            "Aggregate changed; reload before editing",
            Some(rev),
        ),
        StoreError::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            "Database operation failed",
            None,
        ),
        StoreError::WorkConflict => (
            StatusCode::CONFLICT,
            ErrorCode::WorkConflict,
            "Stream assignment or command state conflict",
            None,
        ),
    };
    (
        status,
        Json(ApiError {
            code,
            message: message.into(),
            request_id,
            current_revision: revision,
        }),
    )
        .into_response()
}
async fn db_call<T, F>(request: RequestId, change: F) -> Response
where
    T: serde::Serialize + Send + 'static,
    F: FnOnce() -> Result<T, StoreError> + Send + 'static,
{
    match tokio::task::spawn_blocking(change).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(err)) => db_error(err, request),
        Err(_) => db_error(StoreError::Internal, request),
    }
}
async fn update_project(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    body: Result<Json<UpdateProject>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid project update JSON",
        );
    };
    db_call(body.request_id, move || app.store.update_project(id, body)).await
}

async fn create_project(
    State(app): State<App>,
    body: Result<Json<CreateProject>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid project JSON",
        );
    };
    db_call(body.request_id, move || app.store.create_project(body)).await
}
async fn set_project_mirror(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    body: Result<Json<SetProjectMirror>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid project mirror JSON",
        );
    };
    db_call(body.request_id, move || app.store.set_project_mirror(id, body)).await
}
async fn create_ticket(
    State(app): State<App>,
    body: Result<Json<CreateTicket>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid ticket JSON",
        );
    };
    db_call(body.request_id, move || app.store.create_ticket(body)).await
}
async fn update_ticket(
    State(app): State<App>,
    Path(id): Path<String>,
    body: Result<Json<UpdateTicket>, JsonRejection>,
) -> Response {
    let Ok(id) = id.parse::<TicketId>() else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid ticket ID",
        );
    };
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid ticket JSON",
        );
    };
    db_call(body.request_id, move || app.store.update_ticket(id, body)).await
}

async fn reconcile_publication(
    State(app): State<App>,
    axum::Extension(principal): axum::Extension<Principal>,
    Path(id): Path<TicketId>,
    body: Result<Json<ReconcilePublication>, JsonRejection>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return error(StatusCode::FORBIDDEN, ErrorCode::Unauthorized, "Admin attestation required");
    }
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, ErrorCode::InvalidRequest, "Invalid publication attestation");
    };
    db_call(body.request_id, move || app.store.reconcile_publication(id, body)).await
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EventQuery {
    #[serde(default)]
    after: u64,
    #[serde(default = "event_limit")]
    limit: u32,
}
fn event_limit() -> u32 {
    100
}
async fn events(
    State(app): State<App>,
    query: Result<Query<EventQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let Ok(Query(query)) = query else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid event query",
        );
    };
    db_call(RequestId::new(), move || {
        app.store.events(query.after, query.limit)
    })
    .await
}
