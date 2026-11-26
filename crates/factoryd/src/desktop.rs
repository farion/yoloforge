use super::*;

pub(super) async fn runner_restart_status(
    State(app): State<App>, Path(id): Path<RunnerId>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return error(StatusCode::FORBIDDEN, ErrorCode::Unauthorized, "Admin access required for runner restart");
    }
    Json(restart_status(&app, id).await).into_response()
}

async fn restart_status(app: &App, id: RunnerId) -> RunnerRestartStatus {
    let registry = app.registry.read().await;
    let entry = registry.entries.get(&id);
    RunnerRestartStatus {
        instance_id: entry.map(|e| e.input.instance_id),
        ready: entry.is_some_and(|e| e.seen.elapsed() < SUSPECT_AFTER && matches!(e.input.opencode, OpenCodeStatus::Ready { .. })),
    }
}

pub(super) async fn hold_runner_restart(
    State(app): State<App>, Path(id): Path<RunnerId>,
    axum::Extension(principal): axum::Extension<Principal>, Json(body): Json<RunnerRestartHold>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return error(StatusCode::FORBIDDEN, ErrorCode::Unauthorized, "Admin access required for runner restart");
    }
    let previous = restart_status(&app, id).await;
    match tokio::task::spawn_blocking(move || app.store.hold_runner_restart(id, body.request_id)).await {
        Ok(Ok(())) => Json(previous).into_response(),
        Ok(Err(error)) => db_error(error, body.request_id),
        Err(_) => db_error(StoreError::Internal, body.request_id),
    }
}

pub(super) async fn release_runner_restart(
    State(app): State<App>, Path(id): Path<RunnerId>,
    axum::Extension(principal): axum::Extension<Principal>, Json(body): Json<RunnerRestartHold>,
) -> Response {
    if principal.role != DeviceRole::Admin {
        return error(StatusCode::FORBIDDEN, ErrorCode::Unauthorized, "Admin access required for runner restart");
    }
    db_call(body.request_id, move || app.store.release_runner_restart(id, body.request_id)).await
}

pub(super) async fn retry_task(
    State(app): State<App>, Path(id): Path<TicketId>,
    Json(body): Json<RetryTask>,
) -> Response {
    db_call(body.request_id, move || app.store.retry_task(id, body)).await
}

pub(super) async fn accept_intent_solution(
    State(app): State<App>, Path(id): Path<TicketId>,
    Json(body): Json<AcceptIntentSolution>,
) -> Response {
    db_call(body.request_id, move || app.store.accept_intent_solution(id, body)).await
}

pub(super) async fn return_intent_to_draft(
    State(app): State<App>, Path(id): Path<TicketId>,
    Json(body): Json<ReturnIntentToDraft>,
) -> Response {
    let supported: Vec<_> = app.registry.read().await.entries.iter().filter(|(_, entry)|
        entry.input.capabilities.iter().any(|cap| cap == MERGER_DRAFT_RESET_CAPABILITY)
            && entry.input.capabilities.iter().any(|cap| cap == DRAFT_RESET_CAPABILITY))
        .map(|(id, _)| *id).collect();
    db_call(body.request_id, move || app.store.return_intent_to_draft_with_mergers(id, body, &supported)).await
}

pub(super) async fn delete_failed_task(
    State(app): State<App>, Path(id): Path<TicketId>, Json(body): Json<DeleteFailedTask>,
) -> Response {
    db_call(body.request_id, move || app.store.delete_failed_task(id, body)).await
}

pub(super) async fn delete_inactive_task(
    State(app): State<App>, Path(id): Path<TicketId>, Json(body): Json<DeleteInactiveTask>,
) -> Response {
    db_call(body.request_id, move || app.store.delete_inactive_task(id, body)).await
}

pub(super) async fn delete_task(
    State(app): State<App>, Path(id): Path<TicketId>, Json(body): Json<DeleteTask>,
) -> Response {
    db_call(body.request_id, move || app.store.delete_task(id, body)).await
}

pub(super) async fn observe_client(app: &App, principal: &Principal, headers: &HeaderMap, ip: Option<std::net::IpAddr>) {
    if principal.role == DeviceRole::Runner {
        return;
    }
    let Some(id) = headers
        .get("x-factory-client-id")
        .and_then(|v| v.to_str().ok())
        .filter(|id| id.parse::<RequestId>().is_ok())
    else {
        return;
    };
    let kind = headers
        .get("x-factory-client-kind")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("api");
    if kind == "runner" {
        return;
    }
    let name = match kind {
        "desktop" => "Desktop",
        "android" => "Android",
        "tui" => "CLI / TUI",
        _ => "API-Client",
    };
    // Scope claimed client IDs to the authenticated identity, never to a credential.
    let key = format!(
        "{}:{id}",
        principal
            .device_id
            .map_or_else(|| "legacy".into(), |id| id.to_string())
    );
    let now = unix_ms();
    let mut registry = app.registry.write().await;
    registry
        .clients
        .retain(|_, c| now.saturating_sub(c.observed_unix_ms) < 300_000);
    if registry.clients.len() >= 512 && !registry.clients.contains_key(&key) {
        if let Some(oldest) = registry
            .clients
            .iter()
            .min_by_key(|(_, c)| c.observed_unix_ms)
            .map(|(id, _)| id.clone())
        {
            registry.clients.remove(&oldest);
        }
    }
    registry.clients.insert(
        key.clone(),
        ClientPresence {
            version: headers.get("x-factory-client-version").and_then(|v| v.to_str().ok())
                .filter(|v| !v.is_empty() && v.len() <= 80
                    && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))).map(str::to_string),
            protocol_version: headers.get(VERSION_HEADER).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()),
            ip,
            id: key,
            name: name.into(),
            device_id: principal.device_id,
            observed_unix_ms: now,
        },
    );
}

pub(super) async fn clients(State(app): State<App>) -> Response {
    let now = unix_ms();
    let registry = app.registry.read().await;
    Json(
        registry
            .clients
            .values()
            .filter(|c| now.saturating_sub(c.observed_unix_ms) < 300_000)
            .cloned()
            .collect::<Vec<_>>(),
    )
    .into_response()
}

#[cfg(test)]
mod client_tests {
    use super::*;

    #[tokio::test]
    async fn client_registry_is_bounded_expires_and_excludes_runner_credentials() {
        let app = App {
            registry: Arc::default(),
            token_hash: None,
            boot_id: BootId::new(),
            store: Store::open(std::path::Path::new(":memory:")).unwrap(),
            stopping: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            mcp_probes: Default::default(),
        };
        let operator = Principal {
            role: DeviceRole::Operator,
            runner_id: None,
            device_id: Some(DeviceId::new()),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-factory-client-id", "invalid".parse().unwrap());
        observe_client(&app, &operator, &headers, None).await;
        assert!(app.registry.read().await.clients.is_empty());
        for _ in 0..520 {
            headers.insert(
                "x-factory-client-id",
                RequestId::new().to_string().parse().unwrap(),
            );
            observe_client(&app, &operator, &headers, Some("192.0.2.10".parse().unwrap())).await;
        }
        assert_eq!(app.registry.read().await.clients.len(), 512);
        assert!(app.registry.read().await.clients.values()
            .all(|client| client.ip == Some("192.0.2.10".parse().unwrap())));
        {
            let mut registry = app.registry.write().await;
            for client in registry.clients.values_mut() {
                client.observed_unix_ms = 0;
            }
        }
        observe_client(&app, &operator, &headers, None).await;
        assert_eq!(app.registry.read().await.clients.len(), 1);
        let runner = Principal {
            role: DeviceRole::Runner,
            runner_id: Some(RunnerId::new()),
            device_id: Some(DeviceId::new()),
        };
        observe_client(&app, &runner, &headers, None).await;
        assert_eq!(app.registry.read().await.clients.len(), 1);
    }

    #[test]
    fn peer_ip_comes_from_the_socket_not_untrusted_forwarded_headers() {
        let mut request = axum::http::Request::builder()
            .header("x-forwarded-for", "192.0.2.99")
            .header("forwarded", "for=192.0.2.98")
            .body(axum::body::Body::empty()).unwrap();
        assert_eq!(observed_ip(&request), None);
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "[::ffff:192.0.2.10]:1234".parse::<std::net::SocketAddr>().unwrap()));
        assert_eq!(observed_ip(&request), Some("192.0.2.10".parse().unwrap()));
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "[2001:db8::42]:1234".parse::<std::net::SocketAddr>().unwrap()));
        assert_eq!(observed_ip(&request), Some("2001:db8::42".parse().unwrap()));
    }
}
#[derive(serde::Deserialize)]
pub(super) struct DesktopEventQuery {
    #[serde(default)]
    after: u64,
}
pub(super) async fn desktop_events(
    State(app): State<App>,
    Query(query): Query<DesktopEventQuery>,
) -> Response {
    db_call(RequestId::new(), move || {
        app.store.desktop_events(query.after)
    })
    .await
}
pub(super) async fn event_payload(State(app): State<App>, Path(id): Path<u64>) -> Response {
    db_call(RequestId::new(), move || app.store.event_payload(id)).await
}
pub(super) async fn task_payload(State(app): State<App>, Path(id): Path<TicketId>) -> Response {
    db_call(RequestId::new(), move || app.store.task_payload(id)).await
}
pub(super) async fn task_history(
    State(app): State<App>, Path(id): Path<TicketId>, Query(query): Query<NotificationQuery>,
) -> Response {
    db_call(RequestId::new(), move || app.store.task_history(id, query.before)).await
}
pub(super) async fn begin_task_edit(
    State(app): State<App>,
    Path(id): Path<TicketId>,
    body: Result<Json<BeginTaskEdit>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid task edit request",
        );
    };
    db_call(body.request_id, move || app.store.begin_task_edit(id, body)).await
}
pub(super) async fn command_payload(State(app): State<App>, Path(id): Path<CommandId>) -> Response {
    db_call(RequestId::new(), move || app.store.command_payload(id)).await
}
#[derive(serde::Deserialize)]
pub(super) struct ArchiveWindowQuery {
    #[serde(default)]
    conversation: bool,
    after: Option<u64>,
    before: Option<u64>,
}
pub(super) async fn archive_window(
    State(app): State<App>,
    Path(id): Path<CommandId>,
    Query(query): Query<ArchiveWindowQuery>,
) -> Response {
    db_call(RequestId::new(), move || {
        if query.conversation {
            app.store.conversation_window(id, query.after, query.before)
        } else {
            app.store.archive_window(id, query.after, query.before)
        }
    })
    .await
}
pub(super) async fn me(axum::Extension(p): axum::Extension<Principal>) -> Response {
    Json(AccessView {
        role: p.role,
        device_id: p.device_id,
        runner_id: p.runner_id,
    })
    .into_response()
}
pub(super) async fn runtime_binding(State(app): State<App>, Path(id): Path<CommandId>) -> Response {
    db_call(RequestId::new(), move || app.store.runtime_binding(id)).await
}

pub(super) async fn instances(State(app): State<App>) -> Response {
    let registry = app.registry.read().await;
    let now = unix_ms();
    Json(
        registry
            .instances
            .values()
            .flatten()
            .filter(|i| {
                now.saturating_sub(i.observed_unix_ms) < 30_000
                    && registry
                        .entries
                        .get(&i.runner_id)
                        .is_some_and(|e| e.seen.elapsed() < SUSPECT_AFTER)
            })
            .cloned()
            .collect::<Vec<_>>(),
    )
    .into_response()
}

pub(super) async fn report_instances(
    State(app): State<App>,
    Path(runner): Path<RunnerId>,
    headers: HeaderMap,
    body: Result<Json<ReportInstances>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid presence report",
        );
    };
    let guard = match runner_access(&app, runner, &headers).await {
        Ok(g) => g,
        Err(e) => return *e,
    };
    if body.instances.len() > 256
        || body
            .instances
            .iter()
            .map(|i| i.commands.len())
            .sum::<usize>()
            > 8192
        || body.instances.iter().any(|i| {
            i.runner_id != runner
                || i.id.len() != 64
                || !i.id.bytes().all(|b| b.is_ascii_hexdigit())
                || i.commands.len() > 1024
        })
    {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid instance identity",
        );
    }
    let registration = guard.entries[&runner].registration;
    drop(guard);
    let store = app.store.clone();
    let ids = body
        .instances
        .iter()
        .flat_map(|i| i.commands.iter().copied())
        .collect::<Vec<_>>();
    match tokio::task::spawn_blocking(move || store.validate_instance_commands(runner, &ids)).await
    {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return db_error(e, RequestId::new()),
        Err(_) => return db_error(StoreError::Internal, RequestId::new()),
    }
    let mut registry = app.registry.write().await;
    if registry
        .entries
        .get(&runner)
        .is_none_or(|e| e.registration != registration)
    {
        return error(
            StatusCode::CONFLICT,
            ErrorCode::RegistrationExpired,
            "Runner registration changed",
        );
    }
    let now = unix_ms();
    let values = body
        .instances
        .into_iter()
        .map(|mut i| {
            i.observed_unix_ms = now;
            i
        })
        .collect();
    registry.instances.insert(runner, values);
    Json(()).into_response()
}

#[derive(serde::Deserialize)]
pub(super) struct NotificationQuery {
    before: Option<u64>,
}
pub(super) async fn event_history(
    State(app): State<App>,
    Query(query): Query<NotificationQuery>,
) -> Response {
    db_call(RequestId::new(), move || {
        app.store.event_history(query.before)
    })
    .await
}

pub(super) async fn notifications(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<Principal>,
    Query(query): Query<NotificationQuery>,
) -> Response {
    let owner = p
        .device_id
        .map_or_else(|| "bootstrap".to_string(), |id| id.to_string());
    db_call(RequestId::new(), move || {
        app.store.notifications(&owner, query.before)
    })
    .await
}
pub(super) async fn mark_notifications(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<Principal>,
    body: Result<Json<MarkNotifications>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid read receipt",
        );
    };
    let owner = p
        .device_id
        .map_or_else(|| "bootstrap".to_string(), |id| id.to_string());
    db_call(body.request_id, move || {
        app.store.mark_notifications(&owner, body)
    })
    .await
}
pub(super) async fn notification_states(
    State(app): State<App>,
    axum::Extension(p): axum::Extension<Principal>,
    body: Result<Json<NotificationReadQuery>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid notification status query",
        );
    };
    let owner = p
        .device_id
        .map_or_else(|| "bootstrap".to_string(), |id| id.to_string());
    db_call(RequestId::new(), move || {
        app.store.notification_states(&owner, &body.ids)
    })
    .await
}
