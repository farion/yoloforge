use super::*;
pub(super) async fn read(State(app): State<App>, Path(id): Path<CommandId>) -> Response {
    db_call(RequestId::new(), move || app.store.termination(id)).await
}
pub(super) async fn create(
    State(app): State<App>,
    Path(id): Path<CommandId>,
    body: Result<Json<RequestTermination>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid runtime termination request",
        );
    };
    db_call(body.request_id, move || {
        app.store.request_termination(id, body)
    })
    .await
}
#[derive(serde::Deserialize)]
pub(super) struct QueryEpoch {
    epoch: u64,
}
pub(super) async fn runner_read(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    Query(query): Query<QueryEpoch>,
) -> Response {
    let guard = match work_routes::runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    if !guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == RUNTIME_TERMINATION_CAPABILITY)
    {
        return error(
            StatusCode::CONFLICT,
            ErrorCode::WorkConflict,
            "Runner lacks runtime termination capability",
        );
    }
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.runner_termination(runner, id, query.epoch)
    })
    .await
}
pub(super) async fn report(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<ReportTermination>, JsonRejection>,
) -> Response {
    let guard = match work_routes::runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    if !guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == RUNTIME_TERMINATION_CAPABILITY)
    {
        return error(
            StatusCode::CONFLICT,
            ErrorCode::WorkConflict,
            "Runner lacks runtime termination capability",
        );
    }
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid removal report",
        );
    };
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.report_termination(runner, id, body)
    })
    .await
}
