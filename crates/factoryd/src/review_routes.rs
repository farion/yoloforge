use super::*;
pub(super) async fn hosted(
    State(app): State<App>,
    Path(id): Path<CommandId>,
    body: Result<Json<ReportHostedReview>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid hosted review receipt",
        );
    };
    db_call(body.request_id, move || {
        app.store.report_hosted_review(id, body)
    })
    .await
}
pub(super) async fn session_limit(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    body: Result<Json<SetSessionLimit>, JsonRejection>,
) -> Response {
    let Ok(Json(b)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid session limit",
        );
    };
    db_call(b.request_id, move || app.store.set_session_limit(id, b)).await
}
pub(super) async fn pause_project(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    body: Result<Json<PauseProject>, JsonRejection>,
) -> Response {
    let Ok(Json(b)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid project pause",
        );
    };
    db_call(b.request_id, move || app.store.pause_project(id, b)).await
}
pub(super) async fn task(
    State(app): State<App>,
    body: Result<Json<SaveTask>, JsonRejection>,
) -> Response {
    let Ok(Json(b)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid task prompt/images",
        );
    };
    db_call(b.request_id, move || app.store.save_task(b)).await
}
pub(super) async fn setup(
    State(app): State<App>,
    body: Result<Json<SetupProject>, JsonRejection>,
) -> Response {
    let Ok(Json(b)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid project setup",
        );
    };
    db_call(b.request_id, move || app.store.setup_project(b)).await
}
pub(super) async fn review(
    State(app): State<App>,
    Path(id): Path<StreamId>,
    body: Result<Json<RequestReview>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid review request",
        );
    };
    db_call(body.request_id, move || app.store.request_review(id, body)).await
}
pub(super) async fn integrate(
    State(app): State<App>,
    Path(id): Path<StreamId>,
    body: Result<Json<RequestIntegration>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid integration request",
        );
    };
    db_call(body.request_id, move || {
        app.store.request_integration(id, body)
    })
    .await
}
pub(super) async fn read(State(app): State<App>, Path(id): Path<CommandId>) -> Response {
    db_call(RequestId::new(), move || app.store.review_job(id)).await
}
#[derive(serde::Deserialize)]
pub(super) struct Epoch {
    epoch: u64,
}
pub(super) async fn runner_read(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    Query(q): Query<Epoch>,
) -> Response {
    let guard = match work_routes::runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    if !guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == REVIEW_CAPABILITY)
    {
        return error(
            StatusCode::CONFLICT,
            ErrorCode::WorkConflict,
            "Runner lacks review capability",
        );
    }
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.runner_review_job(runner, id, q.epoch)
    })
    .await
}
