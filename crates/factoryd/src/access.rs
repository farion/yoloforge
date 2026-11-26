use super::*;
#[derive(Clone)]
pub(super) struct Principal {
    pub role: DeviceRole,
    pub runner_id: Option<RunnerId>,
    pub device_id: Option<DeviceId>,
}
impl Principal {
    pub fn allows(&self, path: &str) -> bool {
        match self.role {
            DeviceRole::Admin => true,
            DeviceRole::Operator => {
                !path.starts_with("/v1/devices") && !path.starts_with("/v1/runners")
            }
            DeviceRole::Runner => {
                path == "/v1/runners/register"
                    || self
                        .runner_id
                        .is_some_and(|id| path.starts_with(&format!("/v1/runners/{id}/")))
            }
        }
    }
    pub fn active(&self, store: &Store) -> bool {
        self.device_id.is_none_or(|id| {
            store
                .devices()
                .is_ok_and(|devices| devices.iter().any(|d| d.id == id && !d.revoked))
        })
    }
}
pub(super) async fn health(State(app): State<App>) -> Response {
    let store = app.store.clone();
    match tokio::task::spawn_blocking(move || store.events(0, 1)).await {
        Ok(Ok(_)) => (
            StatusCode::OK,
            Json(serde_json::json!({"status":"ready","version":env!("CARGO_PKG_VERSION")})),
        )
            .into_response(),
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Internal,
            "Database not ready",
        ),
    }
}
pub(super) async fn issue_device(
    State(app): State<App>,
    body: Result<Json<CreateDevice>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid device request",
        );
    };
    let mut response = db_call(RequestId::new(), move || app.store.create_device(body)).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
pub(super) async fn list_devices(State(app): State<App>) -> Response {
    db_call(RequestId::new(), move || app.store.devices()).await
}
pub(super) async fn revoke(State(app): State<App>, Path(id): Path<DeviceId>) -> Response {
    db_call(RequestId::new(), move || app.store.revoke_device(id)).await
}
