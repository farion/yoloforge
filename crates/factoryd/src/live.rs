use super::*;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::{collections::VecDeque, convert::Infallible, time::Duration};
#[derive(serde::Deserialize)]
pub(super) struct EventCursor {
    #[serde(default)]
    after: u64,
}
struct Feed {
    app: App,
    principal: Principal,
    cursor: u64,
    pending: VecDeque<FactoryEvent>,
}
pub(super) async fn event_stream(
    State(app): State<App>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<EventCursor>,
) -> Response {
    let cursor = match headers.get("last-event-id") {
        Some(v) => match v.to_str().ok().and_then(|s| s.parse::<u64>().ok()) {
            Some(v) => v,
            None => {
                return error(
                    StatusCode::BAD_REQUEST,
                    ErrorCode::InvalidRequest,
                    "Invalid event cursor",
                )
            }
        },
        None => query.after,
    };
    if cursor > i64::MAX as u64 {
        return error(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidRequest,
            "Invalid event cursor",
        );
    }
    let stream = futures_util::stream::unfold(
        Feed {
            app,
            principal,
            cursor,
            pending: VecDeque::new(),
        },
        |mut feed| async move {
            loop {
                if feed.app.stopping.load(std::sync::atomic::Ordering::Relaxed) {
                    return None;
                }
                let store = feed.app.store.clone();
                let principal = feed.principal.clone();
                if !tokio::task::spawn_blocking(move || principal.active(&store))
                    .await
                    .unwrap_or(false)
                {
                    return None;
                }
                if let Some(event) = feed.pending.pop_front() {
                    feed.cursor = event.sequence;
                    let output = Event::default()
                        .id(event.sequence.to_string())
                        .event("factory")
                        .json_data(&event)
                        .ok()?;
                    return Some((Ok::<_, Infallible>(output), feed));
                }
                let store = feed.app.store.clone();
                let cursor = feed.cursor;
                let page = tokio::task::spawn_blocking(move || store.events(cursor, 100))
                    .await
                    .ok()?
                    .ok()?;
                feed.pending = page.events.into();
                if feed.pending.is_empty() {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        },
    );
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
        .into_response()
}
