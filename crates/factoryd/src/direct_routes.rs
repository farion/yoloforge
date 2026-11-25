use super::*;
use factory_protocol::directs::*;

pub(super) async fn list(State(app): State<App>, Path(project): Path<ProjectId>) -> Response {
    db_call(RequestId::new(), move || app.store.list_directs(project)).await
}

pub(super) async fn create(State(app): State<App>, body: Result<Json<CreateDirect>, JsonRejection>) -> Response {
    let Ok(Json(body)) = body else { return invalid(); };
    db_call(body.request_id, move || app.store.create_direct(body)).await
}

pub(super) async fn prompt(State(app): State<App>, Path(id): Path<CommandId>, body: Result<Json<PromptDirect>, JsonRejection>) -> Response {
    let Ok(Json(body)) = body else { return invalid(); };
    let registry = app.registry.read().await;
    let supported = body.selection.as_ref().is_none_or(|selection| registry.entries.get(&selection.runner_id)
        .is_some_and(|runner| runner.input.capabilities.iter().any(|cap| cap == CAPABILITY)));
    drop(registry);
    db_call(body.request_id, move || app.store.prompt_direct_with_runner_support(id, body, supported)).await
}

pub(super) async fn archive(State(app): State<App>, Path(id): Path<CommandId>, body: Result<Json<SetDirectArchived>, JsonRejection>) -> Response {
    let Ok(Json(body)) = body else { return invalid(); };
    db_call(body.request_id, move || app.store.set_direct_archived(id, body)).await
}

fn invalid() -> Response {
    error(StatusCode::BAD_REQUEST, ErrorCode::InvalidRequest, "Invalid Direct request")
}
