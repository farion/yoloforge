use super::*;
pub(super) async fn read(State(app): State<App>, Path(id): Path<KnowledgeId>) -> Response {
    db_call(RequestId::new(), move || app.store.knowledge_note(id)).await
}
pub(super) async fn create(
    State(app): State<App>,
    body: Result<Json<CreateKnowledge>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid knowledge request",
        );
    };
    db_call(body.request_id, move || app.store.create_knowledge(body)).await
}
pub(super) async fn retire(
    State(app): State<App>,
    Path(id): Path<KnowledgeId>,
    body: Result<Json<RetireKnowledge>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid retirement request",
        );
    };
    db_call(body.request_id, move || {
        app.store.retire_knowledge(id, body)
    })
    .await
}
pub(super) async fn list(State(app): State<App>, Path(id): Path<ProjectId>) -> Response {
    db_call(RequestId::new(), move || app.store.knowledge(id)).await
}
#[derive(serde::Deserialize)]
pub(super) struct ContextQuery {
    branch: Option<String>,
    ticket: Option<TicketId>,
    #[serde(default = "default_budget")]
    budget_bytes: usize,
}
fn default_budget() -> usize {
    8192
}
pub(super) async fn context(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    Query(query): Query<ContextQuery>,
) -> Response {
    db_call(RequestId::new(), move || {
        app.store.knowledge_context_for_branch(
            id,
            query.ticket,
            query.branch.as_deref(),
            query.budget_bytes,
        )
    })
    .await
}
