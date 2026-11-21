use super::*;
pub(super) async fn read(State(app): State<App>, Path(id): Path<ProjectId>) -> Response {
    db_call(RequestId::new(), move || app.store.scheduler_policy(id)).await
}
pub(super) async fn configure(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    body: Result<Json<SetScheduler>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid scheduler policy",
        );
    };
    db_call(body.request_id, move || app.store.set_scheduler(id, body)).await
}
pub(super) async fn dependencies(
    State(app): State<App>,
    Path(id): Path<TicketId>,
    body: Result<Json<SetDependencies>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid ticket dependencies",
        );
    };
    db_call(body.request_id, move || {
        app.store.set_dependencies(id, body)
    })
    .await
}
