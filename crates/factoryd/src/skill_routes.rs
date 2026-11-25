use super::*;
use factory_protocol::skills::*;

pub(super) async fn catalog(State(app): State<App>) -> Response {
    db_call(RequestId::new(), move || app.store.factory_skill_catalog()).await
}

pub(super) async fn import(
    State(app): State<App>,
    axum::Extension(principal): axum::Extension<Principal>,
    body: Result<Json<ImportFactorySkill>, JsonRejection>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return error(
            StatusCode::FORBIDDEN,
            ErrorCode::Unauthorized,
            "Nur Factory-Admins können Skills importieren",
        );
    }
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Ungültiges Skill-Paket",
        );
    };
    db_call(body.request_id, move || {
        app.store.import_factory_skill(body)
    })
    .await
}

pub(super) async fn bundle(
    State(app): State<App>,
    Path((runner, command, id, version)): Path<(RunnerId, CommandId, String, u32)>,
    headers: HeaderMap,
) -> Response {
    let guard = match runner_access(&app, runner, &headers).await {
        Ok(guard) => guard,
        Err(response) => return *response,
    };
    if !guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|cap| cap == FACTORY_SKILLS_CAPABILITY)
    {
        return error(
            StatusCode::FORBIDDEN,
            ErrorCode::Unauthorized,
            "Runner unterstützt keine Factory-Skills",
        );
    }
    drop(guard);
    db_call(RequestId::new(), move || {
        app.store
            .factory_skill_for_command(runner, command, FactorySkillRef { id, version })
    })
    .await
}

pub(super) async fn read(State(app): State<App>, Path(id): Path<ProjectId>) -> Response {
    db_call(RequestId::new(), move || app.store.project_skills(id)).await
}

pub(super) async fn save(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    axum::Extension(principal): axum::Extension<Principal>,
    body: Result<Json<SetProjectSkills>, JsonRejection>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return error(
            StatusCode::FORBIDDEN,
            ErrorCode::Unauthorized,
            "Nur Factory-Admins können Skills freigeben",
        );
    }
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Ungültige Skill-Konfiguration",
        );
    };
    db_call(body.request_id, move || {
        app.store.set_project_skills(id, body)
    })
    .await
}
