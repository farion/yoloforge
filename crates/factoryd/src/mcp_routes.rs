use super::*;
use factory_protocol::mcp::*;

pub(super) async fn catalog(
    State(app): State<App>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return admin_only();
    }
    db_call(RequestId::new(), move || app.store.local_mcp_catalog()).await
}

pub(super) async fn choices(State(app): State<App>) -> Response {
    db_call(RequestId::new(), move || {
        let local = app
            .store
            .local_mcp_catalog()?
            .entries
            .into_iter()
            .map(|entry| serde_json::json!({"id": entry.id, "name": entry.name, "remote": false}));
        let remote = app.store.remote_mcps()?.into_iter().map(|entry|
            serde_json::json!({
                "id": entry.reference.id, "name": entry.reference.name,
                "remote": true, "revision": entry.reference.revision,
            }));
        Ok(local.chain(remote).collect::<Vec<_>>())
    })
    .await
}

pub(super) async fn save_catalog(
    State(app): State<App>,
    axum::Extension(principal): axum::Extension<Principal>,
    body: Result<Json<SetLocalMcpCatalog>, JsonRejection>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return admin_only();
    }
    let Ok(Json(body)) = body else {
        return invalid();
    };
    db_call(body.request_id, move || {
        app.store.save_local_mcp_catalog(body)
    })
    .await
}

pub(super) async fn project(State(app): State<App>, Path(id): Path<ProjectId>) -> Response {
    db_call(RequestId::new(), move || app.store.project_local_mcps(id)).await
}

pub(super) async fn save_project(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    axum::Extension(principal): axum::Extension<Principal>,
    body: Result<Json<SetProjectLocalMcps>, JsonRejection>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return admin_only();
    }
    let Ok(Json(body)) = body else {
        return invalid();
    };
    db_call(body.request_id, move || {
        app.store.save_project_local_mcps(id, body)
    })
    .await
}

fn admin_only() -> Response {
    error(
        StatusCode::FORBIDDEN,
        ErrorCode::Unauthorized,
        "Nur Factory-Admins können lokale MCPs verwalten",
    )
}

fn invalid() -> Response {
    error(
        StatusCode::BAD_REQUEST,
        ErrorCode::InvalidRequest,
        "Ungültige MCP-Konfiguration",
    )
}
