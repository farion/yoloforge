use super::*;
#[derive(serde::Deserialize)]
pub(super) struct Cursor {
    #[serde(default)]
    after: u64,
    through_revision: Option<u64>,
}
pub(super) async fn read(
    State(app): State<App>,
    Path(id): Path<CommandId>,
    Query(query): Query<Cursor>,
) -> Response {
    db_call(RequestId::new(), move || {
        app.store.archive(id, query.after, query.through_revision)
    })
    .await
}
pub(super) async fn write(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<ReportArchive>, JsonRejection>,
) -> Response {
    let guard = match work_routes::runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid archive batch",
        );
    };
    let store = app.store.clone();
    let response = db_call(RequestId::new(), move || {
        store.report_archive(runner, id, body)
    })
    .await;
    drop(guard);
    response
}
