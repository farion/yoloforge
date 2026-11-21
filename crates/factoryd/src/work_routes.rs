use super::*;

pub(super) async fn command_excerpt(State(app): State<App>, Path(id): Path<CommandId>) -> Response {
    db_call(RequestId::new(), move || app.store.excerpt(id)).await
}
pub(super) async fn report_excerpt(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<ReportExcerpt>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid session excerpt",
        );
    };
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.report_excerpt(runner, id, body)
    })
    .await
}

pub(super) async fn set_control(
    State(app): State<App>,
    Path(id): Path<StreamId>,
    body: Result<Json<SetControl>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid stream control",
        );
    };
    db_call(body.request_id, move || app.store.set_control(id, body)).await
}
#[derive(serde::Deserialize)]
pub(super) struct ControlQuery {
    epoch: u64,
}
pub(super) async fn runner_control(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    Query(query): Query<ControlQuery>,
) -> Response {
    let guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let freezer = guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == CONTAINER_FREEZE_CAPABILITY);
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        let control = store.runner_control(runner, id, query.epoch)?;
        if !freezer && control.as_ref().is_some_and(|c| c.proof.is_some()) {
            return Err(store::StoreError::WorkConflict);
        }
        Ok(control)
    })
    .await
}
pub(super) async fn report_control(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<ReportControl>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid control report",
        );
    };
    let guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    if (body.proof.is_some()
        || matches!(
            body.state,
            PauseState::Frozen | PauseState::ResumeRequested | PauseState::Released
        ))
        && !guard.entries[&runner]
            .input
            .capabilities
            .iter()
            .any(|c| c == CONTAINER_FREEZE_CAPABILITY)
    {
        return error(
            StatusCode::CONFLICT,
            ErrorCode::WorkConflict,
            "Runner lacks container freezer capability",
        );
    }
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.report_control(runner, id, body)
    })
    .await
}

pub(super) async fn answer_question(
    State(app): State<App>,
    Path(id): Path<QuestionId>,
    body: Result<Json<AnswerQuestion>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid answer JSON",
        );
    };
    db_call(body.request_id, move || app.store.answer_question(id, body)).await
}
pub(super) async fn sync_questions(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<QuestionSnapshot>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid questions JSON",
        );
    };
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.sync_questions(runner, id, body)
    })
    .await
}
pub(super) async fn answer_delivery(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, QuestionId)>,
    headers: HeaderMap,
    body: Result<Json<AnswerDelivery>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid delivery JSON",
        );
    };
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(body.request_id, move || {
        store.answer_delivery(runner, id, body)
    })
    .await
}

pub(super) async fn update_profile(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    body: Result<Json<UpdateProfile>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid profile JSON",
        );
    };
    db_call(body.request_id, move || app.store.update_profile(id, body)).await
}
pub(super) async fn observe_agent(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<ObserveAgent>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid observation JSON",
        );
    };
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.observe_agent(runner, id, body)
    })
    .await
}

pub(super) async fn runner_access<'a>(
    app: &'a App,
    id: RunnerId,
    headers: &HeaderMap,
) -> Result<tokio::sync::RwLockReadGuard<'a, Registry>, Box<Response>> {
    let registration = headers
        .get(RUNNER_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<RegistrationId>().ok());
    let guard = app.registry.read().await;
    let Some(entry) = guard.entries.get(&id) else {
        return Err(Box::new(error(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "Runner must register",
        )));
    };
    if registration != Some(entry.registration) || entry.seen.elapsed() >= UNREACHABLE_AFTER {
        return Err(Box::new(error(
            StatusCode::CONFLICT,
            ErrorCode::RegistrationExpired,
            "Current runner registration required",
        )));
    }
    Ok(guard)
}
pub(super) async fn create_stream(
    State(app): State<App>,
    body: Result<Json<CreateStream>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid stream JSON",
        );
    };
    db_call(body.request_id, move || app.store.create_stream(body)).await
}
pub(super) async fn enqueue(
    State(app): State<App>,
    Path(id): Path<StreamId>,
    body: Result<Json<EnqueueCommand>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid command JSON",
        );
    };
    db_call(body.request_id, move || app.store.enqueue(id, body)).await
}
#[derive(serde::Deserialize)]
pub(super) struct PollQuery {
    #[serde(default)]
    wait_seconds: u8,
}
pub(super) async fn pending_commands(
    State(app): State<App>,
    Path(id): Path<RunnerId>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<PollQuery>,
    headers: HeaderMap,
) -> Response {
    if query.wait_seconds > 20 {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Long poll limited to 20 seconds",
        );
    }
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(query.wait_seconds as u64);
    loop {
        if app.stopping.load(std::sync::atomic::Ordering::Relaxed) {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::Internal,
                "Server stopping",
            );
        }
        let guard = match runner_access(&app, id, &headers).await {
            Ok(g) => g,
            Err(e) => return *e,
        };
        let store = app.store.clone();
        let identity = principal.clone();
        let lifecycle = guard.entries[&id]
            .input
            .capabilities
            .iter()
            .any(|c| c == SESSION_LIFECYCLE_CAPABILITY);
        let result = tokio::task::spawn_blocking(move || {
            if !identity.active(&store) {
                return Err(StoreError::NotFound);
            }
            store.pending(id).map(|mut commands| {
                commands.retain(|c| lifecycle || !c.requires_session_lifecycle());
                commands
            })
        })
        .await;
        drop(guard);
        match result {
            Ok(Ok(commands)) if !commands.is_empty() || tokio::time::Instant::now() >= deadline => {
                return Json(commands).into_response()
            }
            Ok(Ok(_)) => {}
            _ => {
                return error(
                    StatusCode::UNAUTHORIZED,
                    ErrorCode::Unauthorized,
                    "Runner credential unavailable",
                )
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}
pub(super) async fn ack_command(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<AckCommand>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid command acknowledgement",
        );
    };
    let guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    let lifecycle = guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == SESSION_LIFECYCLE_CAPABILITY);
    db_call(body.request_id, move || {
        store.acknowledge_supported(runner, id, body, lifecycle)
    })
    .await
}
pub(super) async fn command_result(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<CommandResult>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid command result",
        );
    };
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(body.request_id, move || store.complete(runner, id, body)).await
}
