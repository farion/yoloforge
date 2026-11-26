use super::*;

pub(super) async fn command_excerpt(State(app): State<App>, Path(id): Path<CommandId>) -> Response {
    db_call(RequestId::new(), move || app.store.excerpt(id)).await
}
pub(super) async fn report_excerpt(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<ReportExcerpt>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid session excerpt",
        );
    };
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.report_excerpt(runner, id, body)
    })
    .await
}

pub(super) async fn set_control(
    State(app): State<App>,
    Path(id): Path<StreamId>,
    body: Result<Json<SetControl>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid stream control",
        );
    };
    db_call(body.request_id, move || app.store.set_control(id, body)).await
}
pub(super) async fn retry_control(
    State(app): State<App>,
    Path(id): Path<StreamId>,
    body: Result<Json<RetryControl>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid control recovery request",
        );
    };
    db_call(body.request_id, move || app.store.retry_control(id, body)).await
}
#[derive(serde::Deserialize)]
pub(super) struct ControlQuery {
    epoch: u64,
}
pub(super) async fn runner_control(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    Query(query): Query<ControlQuery>,
) -> Response {
    let guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let freezer = guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == CONTAINER_FREEZE_CAPABILITY);
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        let control = store.runner_control(runner, id, query.epoch)?;
        if !freezer && control.as_ref().is_some_and(|c| c.proof.is_some()) {
            return Err(store::StoreError::WorkConflict);
        }
        Ok(control)
    })
    .await
}
pub(super) async fn report_control(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<ReportControl>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid control report",
        );
    };
    let guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    if (body.proof.is_some()
        || matches!(
            body.state,
            PauseState::Frozen | PauseState::ResumeRequested | PauseState::Released
        ))
        && !guard.entries[&runner]
            .input
            .capabilities
            .iter()
            .any(|c| c == CONTAINER_FREEZE_CAPABILITY)
    {
        return error(
            StatusCode::CONFLICT,
            ErrorCode::WorkConflict,
            "Runner lacks container freezer capability",
        );
    }
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.report_control(runner, id, body)
    })
    .await
}

pub(super) async fn answer_question(
    State(app): State<App>,
    Path(id): Path<QuestionId>,
    body: Result<Json<AnswerQuestion>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid answer JSON",
        );
    };
    db_call(body.request_id, move || app.store.answer_question(id, body)).await
}
pub(super) async fn sync_questions(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<QuestionSnapshot>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid questions JSON",
        );
    };
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.sync_questions(runner, id, body)
    })
    .await
}
pub(super) async fn answer_delivery(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, QuestionId)>,
    headers: HeaderMap,
    body: Result<Json<AnswerDelivery>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid delivery JSON",
        );
    };
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(body.request_id, move || {
        store.answer_delivery(runner, id, body)
    })
    .await
}

pub(super) async fn update_profile(
    State(app): State<App>,
    Path(id): Path<ProjectId>,
    body: Result<Json<UpdateProfile>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid profile JSON",
        );
    };
    db_call(body.request_id, move || app.store.update_profile(id, body)).await
}
pub(super) async fn observe_agent(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<ObserveAgent>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid observation JSON",
        );
    };
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(RequestId::new(), move || {
        store.observe_agent(runner, id, body)
    })
    .await
}

pub(super) async fn runner_access<'a>(
    app: &'a App,
    id: RunnerId,
    headers: &HeaderMap,
) -> Result<tokio::sync::RwLockReadGuard<'a, Registry>, Box<Response>> {
    let registration = headers
        .get(RUNNER_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<RegistrationId>().ok());
    let guard = app.registry.read().await;
    let Some(entry) = guard.entries.get(&id) else {
        return Err(Box::new(error(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "Runner must register",
        )));
    };
    if registration != Some(entry.registration) || entry.seen.elapsed() >= UNREACHABLE_AFTER {
        return Err(Box::new(error(
            StatusCode::CONFLICT,
            ErrorCode::RegistrationExpired,
            "Current runner registration required",
        )));
    }
    Ok(guard)
}
pub(super) async fn create_stream(
    State(app): State<App>,
    body: Result<Json<CreateStream>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid stream JSON",
        );
    };
    db_call(body.request_id, move || app.store.create_stream(body)).await
}
pub(super) async fn enqueue(
    State(app): State<App>,
    Path(id): Path<StreamId>,
    body: Result<Json<EnqueueCommand>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid command JSON",
        );
    };
    db_call(body.request_id, move || app.store.enqueue(id, body)).await
}
#[derive(serde::Deserialize)]
pub(super) struct PollQuery {
    #[serde(default)]
    wait_seconds: u8,
}
pub(super) async fn pending_commands(
    State(app): State<App>,
    Path(id): Path<RunnerId>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<PollQuery>,
    headers: HeaderMap,
) -> Response {
    if query.wait_seconds > 20 {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Long poll limited to 20 seconds",
        );
    }
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(query.wait_seconds as u64);
    loop {
        if app.stopping.load(std::sync::atomic::Ordering::Relaxed) {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::Internal,
                "Server stopping",
            );
        }
        let guard = match runner_access(&app, id, &headers).await {
            Ok(g) => g,
            Err(e) => return *e,
        };
        let store = app.store.clone();
        let identity = principal.clone();
        let setup = guard.entries[&id]
            .input
            .capabilities
            .iter()
            .any(|c| c == PROJECT_SETUP_CAPABILITY);
        let reviews = guard.entries[&id]
            .input
            .capabilities
            .iter()
            .any(|c| c == REVIEW_CAPABILITY);
        let checkpoints = guard.entries[&id]
            .input
            .capabilities
            .iter()
            .any(|c| c == CHECKPOINT_CAPABILITY);
        let lifecycle = guard.entries[&id]
            .input
            .capabilities
            .iter()
            .any(|c| c == SESSION_LIFECYCLE_CAPABILITY);
        let context_planning = guard.entries[&id]
            .input
            .capabilities
            .iter()
            .any(|c| c == CONTEXT_PLANNING_CAPABILITY);
        let files = guard.entries[&id]
            .input
            .capabilities
            .iter()
            .any(|c| c == TASK_FILES_CAPABILITY);
        let repo_skills = guard.entries[&id].input.capabilities.iter().any(|c| c == REPO_SKILLS_CAPABILITY);
        let factory_skills = guard.entries[&id].input.capabilities.iter().any(|c| c == skills::FACTORY_SKILLS_CAPABILITY);
        let local_mcp = guard.entries[&id].input.capabilities.iter().any(|c| c == mcp::LOCAL_MCP_CAPABILITY);
        let remote_mcp = guard.entries[&id].input.capabilities.iter().any(|c| c == mcp::REMOTE_MCP_CAPABILITY);
        let target_git = guard.entries[&id].input.capabilities.iter().any(|c| c == TARGET_GIT_CAPABILITY);
        let directs = guard.entries[&id].input.capabilities.iter().any(|c| c == factory_protocol::directs::CAPABILITY);
        let merger = guard.entries[&id].input.capabilities.iter().any(|c| c == MERGER_CAPABILITY);
        let merger_recovery = guard.entries[&id].input.capabilities.iter().any(|c| c == MERGER_RECOVERY_CAPABILITY);
        let result = tokio::task::spawn_blocking(move || {
            if !identity.active(&store) {
                return Err(StoreError::NotFound);
            }
            store.pending_with_direct_support(id, directs).map(|mut commands| {
                commands.retain(|c| merger || c.agent_spec.as_ref().is_none_or(|s| s.role != MERGER_ROLE));
                commands.retain(|c| merger_recovery || !is_merger_recovery(c));
                commands
                    .retain(|c| files || c.agent_spec.as_ref().is_none_or(|s| s.files.is_empty()));
                commands.retain(|c| repo_skills || c.agent_spec.as_ref().is_none_or(|s| s.repo_skills.is_none()));
                commands.retain(|c| factory_skills || c.agent_spec.as_ref().is_none_or(|s| s.factory_skills.is_empty()));
                commands.retain(|c| local_mcp || c.agent_spec.as_ref().is_none_or(|s| s.local_mcps.is_empty()));
                commands.retain(|c| remote_mcp || c.agent_spec.as_ref().is_none_or(|s| s.remote_mcps.is_empty()));
                commands.retain(|c| {
                    setup || c.workspace_spec.as_ref().is_none_or(|s| s.setup.is_none())
                });
                commands.retain(|c| lifecycle || !c.requires_session_lifecycle());
                commands.retain(|c| {
                    context_planning
                        || c.agent_spec
                            .as_ref()
                            .is_none_or(|s| s.profile.context_planning.is_none())
                });
                commands.retain(|c| checkpoints || !c.requires_checkpoint());
                commands.retain(|c| target_git || !c.requires_target_git());
                commands.retain(|c| {
                    reviews
                        || !matches!(
                            c.kind,
                            CommandKind::ReviewCheckpoint | CommandKind::IntegrateReview
                        )
                });
                commands
            })
        })
        .await;
        drop(guard);
        match result {
            Ok(Ok(commands)) if !commands.is_empty() || tokio::time::Instant::now() >= deadline => {
                return Json(commands).into_response()
            }
            Ok(Ok(_)) => {}
            _ => {
                return error(
                    StatusCode::UNAUTHORIZED,
                    ErrorCode::Unauthorized,
                    "Runner credential unavailable",
                )
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}
pub(super) async fn ack_command(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<AckCommand>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid command acknowledgement",
        );
    };
    let guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    let lifecycle = guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == SESSION_LIFECYCLE_CAPABILITY);
    let reviews = guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == REVIEW_CAPABILITY);
    let checkpoints = guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == CHECKPOINT_CAPABILITY);
    let setup = guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == PROJECT_SETUP_CAPABILITY);
    let images = guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == TASK_IMAGES_CAPABILITY);
    let context_planning = guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == CONTEXT_PLANNING_CAPABILITY);
    let files = guard.entries[&runner]
        .input
        .capabilities
        .iter()
        .any(|c| c == TASK_FILES_CAPABILITY);
    let repo_skills = guard.entries[&runner].input.capabilities.iter().any(|c| c == REPO_SKILLS_CAPABILITY);
    let factory_skills = guard.entries[&runner].input.capabilities.iter().any(|c| c == skills::FACTORY_SKILLS_CAPABILITY);
    let local_mcp = guard.entries[&runner].input.capabilities.iter().any(|c| c == mcp::LOCAL_MCP_CAPABILITY);
    let remote_mcp = guard.entries[&runner].input.capabilities.iter().any(|c| c == mcp::REMOTE_MCP_CAPABILITY);
    let target_git = guard.entries[&runner].input.capabilities.iter().any(|c| c == TARGET_GIT_CAPABILITY);
    let automatic_checkpoint = guard.entries[&runner].input.capabilities.iter()
        .any(|c| c == AUTOMATIC_CHECKPOINT_CAPABILITY);
    let directs = guard.entries[&runner].input.capabilities.iter().any(|c| c == factory_protocol::directs::CAPABILITY);
    let merger = guard.entries[&runner].input.capabilities.iter().any(|c| c == MERGER_CAPABILITY);
    let merger_recovery = guard.entries[&runner].input.capabilities.iter().any(|c| c == MERGER_RECOVERY_CAPABILITY);
    db_call(body.request_id, move || {
        store.require_direct_support(runner, id, body.ownership_epoch, directs)?;
        store.require_merger_support(runner, id, body.ownership_epoch, merger, merger_recovery)?;
        store.require_context_planning_support(
            runner,
            id,
            body.ownership_epoch,
            context_planning,
        )?;
        store.require_image_support(runner, id, body.ownership_epoch, images)?;
        store.require_file_support(runner, id, body.ownership_epoch, files)?;
        store.require_repo_skill_support(runner, id, body.ownership_epoch, repo_skills)?;
        store.require_factory_skill_support(runner, id, body.ownership_epoch, factory_skills)?;
        store.require_local_mcp_support(runner, id, body.ownership_epoch, local_mcp)?;
        store.require_remote_mcp_support(runner, id, body.ownership_epoch, remote_mcp)?;
        store.require_setup_support(runner, id, body.ownership_epoch, setup)?;
        store.require_checkpoint_support(runner, id, body.ownership_epoch, checkpoints, automatic_checkpoint)?;
        store.require_review_support(runner, id, body.ownership_epoch, reviews)?;
        store.require_target_git_support(runner, id, body.ownership_epoch, target_git)?;
        store.acknowledge_supported(runner, id, body, lifecycle)
    })
    .await
}
pub(super) async fn command_result(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    body: Result<Json<CommandResult>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid command result",
        );
    };
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(body.request_id, move || store.complete(runner, id, body)).await
}

pub(super) async fn publication_recovery(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    Json(body): Json<PublicationRecoveryClaim>,
) -> Response {
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(body.grant_id, move || store.claim_publication_recovery(runner, id, body.grant_id)).await
}

pub(super) async fn release_publication_recovery(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    Json(body): Json<PublicationRecoveryClaim>,
) -> Response {
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(body.grant_id, move || store.release_publication_recovery(runner, id, body.grant_id)).await
}

pub(super) async fn report_recovered_publication(
    State(app): State<App>,
    Path((runner, id)): Path<(RunnerId, CommandId)>,
    headers: HeaderMap,
    Json(body): Json<ReportRecoveredPublication>,
) -> Response {
    let _guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    let store = app.store.clone();
    db_call(body.request_id, move || store.report_recovered_publication(runner, id, body)).await
}
