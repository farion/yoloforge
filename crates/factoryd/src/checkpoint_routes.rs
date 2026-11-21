use super::*;
pub(super) async fn read(State(app): State<App>, Path(id): Path<CommandId>) -> Response {
    db_call(RequestId::new(), move || app.store.checkpoint(id)).await
}
#[derive(serde::Deserialize)]
pub(super) struct Epoch {
    epoch: u64,
}
pub(super) async fn handoff(
    State(app): State<App>,
    Path(id): Path<StreamId>,
    body: Result<Json<HandoffStream>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid handoff",
        );
    };
    db_call(body.request_id, move || app.store.handoff(id, body)).await
}
pub(super) async fn upload(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    Query(query): Query<Epoch>,
    bytes: axum::body::Bytes,
) -> Response {
    let guard = match work_routes::runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    if !guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == CHECKPOINT_CAPABILITY)
    {
        return error(
            StatusCode::CONFLICT,
            ErrorCode::WorkConflict,
            "Runner lacks checkpoint capability",
        );
    }
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.upload_checkpoint_bundle(runner, id, query.epoch, bytes.to_vec())
    })
    .await
}
pub(super) async fn download(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    Query(query): Query<Epoch>,
) -> Response {
    let _guard = match work_routes::runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    match tokio::task::spawn_blocking(move || {
        store.download_checkpoint_bundle(runner, id, query.epoch)
    })
    .await
    {
        Ok(Ok(bytes)) => (
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Ok(Err(StoreError::WorkConflict | StoreError::NotFound)) => error(
            StatusCode::CONFLICT,
            ErrorCode::WorkConflict,
            "Checkpoint unavailable for this command",
        ),
        _ => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            "Cannot read checkpoint bundle",
        ),
    }
}
pub(super) async fn runner_read(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    Query(query): Query<Epoch>,
) -> Response {
    let guard = match work_routes::runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    if !guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == CHECKPOINT_CAPABILITY)
    {
        return error(
            StatusCode::CONFLICT,
            ErrorCode::WorkConflict,
            "Runner lacks checkpoint capability",
        );
    }
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.runner_checkpoint(runner, id, query.epoch)
    })
    .await
}
