use super::*;
pub(super) async fn close_session(
    State(app): State<App>,
    Path(id): Path<CommandId>,
    body: Result<Json<CloseSession>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid session closure request",
        );
    };
    db_call(body.request_id, move || app.store.close_session(id, body)).await
}
pub(super) async fn closure(State(app): State<App>, Path(id): Path<CommandId>) -> Response {
    db_call(RequestId::new(), move || app.store.session_closure(id)).await
}
pub(super) async fn continue_agent(
    State(app): State<App>,
    Path(id): Path<StreamId>,
    body: Result<Json<ContinueAgent>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid continuation request",
        );
    };
    db_call(body.request_id, move || app.store.continue_agent(id, body)).await
}
pub(super) async fn create(
    State(app): State<App>,
    Path(id): Path<CommandId>,
    body: Result<Json<RequestCompaction>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid compaction request",
        );
    };
    db_call(body.request_id, move || {
        app.store.request_compaction(id, body)
    })
    .await
}
pub(super) async fn read(State(app): State<App>, Path(id): Path<CommandId>) -> Response {
    db_call(RequestId::new(), move || app.store.compaction(id)).await
}
#[derive(serde::Deserialize)]
pub(super) struct Epoch {
    epoch: u64,
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
    let store = app.store.clone();
    let response = db_call(RequestId::new(), move || {
        store.runner_compaction(runner, id, query.epoch)
    })
    .await;
    drop(guard);
    response
}
pub(super) async fn report(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<ReportCompaction>, JsonRejection>,
) -> Response {
    let guard = match work_routes::runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid compaction report",
        );
    };
    let store = app.store.clone();
    let response = db_call(RequestId::new(), move || {
        store.report_compaction(runner, id, body)
    })
    .await;
    drop(guard);
    response
}
