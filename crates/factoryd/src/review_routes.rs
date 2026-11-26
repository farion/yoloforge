use super::*;
pub(super) async fn prompt_session(
    State(app): State<App>, Path(root): Path<CommandId>,
    body: Result<Json<PromptSession>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, ErrorCode::InvalidRequest, "Invalid session prompt");
    };
    db_call(body.request_id, move || app.store.prompt_session(root, body)).await
}

pub(super) async fn delete_project(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    body: Result<Json<DeleteProject>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid project deletion",
        );
    };
    db_call(body.request_id, move || app.store.delete_project(id, body)).await
}
pub(super) async fn submit_task(
    State(app): State<App>,
    body: Result<Json<SubmitTask>, JsonRejection>,
) -> Response {
    let Ok(Json(b)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid task submission",
        );
    };
    db_call(b.task.request_id, move || app.store.submit_task(b)).await
}
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
