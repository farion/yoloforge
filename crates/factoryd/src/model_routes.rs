use super::*;
use factory_protocol::models::*;

fn invalid() -> Response {
    error(
        StatusCode::BAD_REQUEST,
        ErrorCode::InvalidRequest,
        "Ungültige Modellkonfiguration",
    )
}

pub(super) async fn factory(State(app): State<App>) -> Response {
    db_call(RequestId::new(), move || app.store.factory_models()).await
}

pub(super) async fn setup(
    gateway: Option<axum::Extension<model_gateway::Gateway>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({"managed": gateway.is_some()}))
}

pub(super) async fn set_factory(
    State(app): State<App>,
    axum::Extension(principal): axum::Extension<Principal>,
    gateway: Option<axum::Extension<model_gateway::Gateway>>,
    body: Result<Json<SetFactoryModels>, JsonRejection>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return error(
            StatusCode::FORBIDDEN,
            ErrorCode::Unauthorized,
            "Nur Factory-Admins können das Standardmodell und Zugänge ändern",
        );
    }
    let Ok(Json(body)) = body else {
        return invalid();
    };
    if let Some(axum::Extension(gateway)) = gateway {
        if let Some(model) = &body.settings.default_model {
            let store = app.store.clone();
            let existing = tokio::task::spawn_blocking(move || store.factory_models()).await;
            let unchanged = existing.is_ok_and(|s| {
                s.is_ok_and(|current| current.default_model.as_ref() == Some(model))
            });
            if !unchanged
                && gateway
                    .access_status(app.store.clone(), model.access_id.clone())
                    .await
                    != "ready"
            {
                return error(
                    StatusCode::CONFLICT,
                    ErrorCode::InvalidRequest,
                    "Den Zugang für das Standardmodell zuerst verbinden",
                );
            }
        }
    }
    db_call(body.request_id, move || app.store.set_factory_models(body)).await
}

pub(super) async fn project(State(app): State<App>, Path(id): Path<ProjectId>) -> Response {
    db_call(RequestId::new(), move || app.store.project_models(id)).await
}

pub(super) async fn set_project(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    body: Result<Json<SetProjectModels>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid();
    };
    db_call(body.request_id, move || {
        app.store.set_project_models(id, body)
    })
    .await
}

pub(super) async fn task(State(app): State<App>, Path(id): Path<TicketId>) -> Response {
    db_call(RequestId::new(), move || app.store.task_models(id)).await
}

pub(super) async fn submit(
    State(app): State<App>,
    body: Result<Json<SubmitTaskWithModel>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid();
    };
    db_call(body.submission.task.request_id, move || {
        app.store.submit_task_with_model(body)
    })
    .await
}
