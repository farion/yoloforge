use super::*;
pub(super) async fn read(State(app): State<App>, Path(id): Path<ProjectId>) -> Response {
    db_call(RequestId::new(), move || app.store.supervisor_policy(id)).await
}
pub(super) async fn configure(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    body: Result<Json<SetSupervisor>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid supervisor configuration",
        );
    };
    db_call(body.request_id, move || app.store.set_supervisor(id, body)).await
}
pub(super) async fn evaluations(State(app): State<App>, Path(id): Path<ProjectId>) -> Response {
    db_call(RequestId::new(), move || {
        app.store.supervisor_evaluations(id)
    })
    .await
}
