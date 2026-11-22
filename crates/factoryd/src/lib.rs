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
pub mod store;
use store::{Store, StoreError};
mod access;
mod archive_routes;
mod checkpoint_routes;
mod compaction_routes;
mod knowledge_routes;
mod live;
mod review_routes;
mod scheduler_routes;
mod supervisor_routes;
mod termination_routes;
mod work_routes;
use access::*;
use live::*;
use work_routes::*;

struct Entry {
    input: RegisterRunner,
    registration: RegistrationId,
    seen: Instant,
    unix_ms: u64,
}
#[derive(Default)]
struct Registry {
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
    };
    Router::new()
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
        .route(
            "/v1/runners/{runner}/commands/{id}/archive",
            axum::routing::put(archive_routes::write)
                .layer(axum::extract::DefaultBodyLimit::max(9 * 1024 * 1024)),
        )
        .route("/v1/knowledge", post(knowledge_routes::create))
        .route("/v1/knowledge/{id}", get(knowledge_routes::read))
        .route("/v1/knowledge/{id}/retire", post(knowledge_routes::retire))
        .route("/v1/projects/{id}/knowledge", get(knowledge_routes::list))
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
        .route("/v1/projects/{id}", patch(update_project))
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
        .route("/v1/tickets", post(create_ticket))
        .route("/v1/tickets/{id}", patch(update_ticket))
        .route("/v1/streams", post(create_stream))
        .route("/v1/streams/{id}/control", post(set_control))
        .route("/v1/projects/setup", post(review_routes::setup))
        .route(
            "/v1/projects/{id}/pause",
            post(review_routes::pause_project),
        )
        .route(
            "/v1/tasks",
            post(review_routes::task).layer(axum::extract::DefaultBodyLimit::max(6 * 1024 * 1024)),
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
        .route(
            "/v1/runners/{runner}/commands/{command}/ack",
            post(ack_command),
        )
        .route(
            "/v1/runners/{runner}/commands/{command}/result",
            post(command_result),
        )
        .route("/v1/events", get(events))
        .route("/v1/events/stream", get(event_stream))
        .route("/v1/runners/register", post(register))
        .route("/v1/runners/{id}/heartbeat", post(heartbeat))
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
    if request.uri().path() == "/healthz" {
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
async fn register(
    State(app): State<App>,
    axum::Extension(principal): axum::Extension<Principal>,
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
        || body.capabilities.len() > 16
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
        if !matches!(
            tokio::task::spawn_blocking(move || {
                store.autostart_runner(runner)?;
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
        if !matches!(
            tokio::task::spawn_blocking(move || {
                store.autostart_runner(id)?;
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
    let store = app.store.clone();
    let durable = match tokio::task::spawn_blocking(move || store.snapshot()).await {
        Ok(Ok(value)) => value,
        _ => return db_error(StoreError::Internal, RequestId::new()),
    };
    let registry = app.registry.read().await;
    Json(Snapshot {
        scheduler_policies: durable.scheduler_policies,
        protocol_version: PROTOCOL_VERSION,
        server_version: env!("CARGO_PKG_VERSION").into(),
        boot_id: app.boot_id,
        revision: registry.revision,
        runners: registry
            .entries
            .iter()
            .map(|(id, e)| RunnerView {
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
