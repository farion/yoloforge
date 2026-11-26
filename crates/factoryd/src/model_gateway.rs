//! Factory model relay. Its keys do not authenticate any control API.
use crate::store::Store;
use anyhow::{ensure, Context, Result};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use factory_protocol::models::{is_copilot_access, COPILOT_INTEGRATION, OPENCODE_FREE_ACCESS};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use subtle::ConstantTimeEq;
use url::Url;

pub(super) mod key_provider;
mod copilot;
pub(super) mod opencode_free;

const CODEX: &str = "https://chatgpt.com/backend-api/codex/responses";

struct Registration {
    url: String,
    password: String,
}

#[derive(Clone)]
pub struct Gateway(Arc<Inner>);

struct Inner {
    central: Registration,
    keys: Vec<[u8; 32]>,
    client: reqwest::Client,
    upstream: String,
    #[cfg(test)]
    copilot_test_upstream: Option<String>,
    pending: tokio::sync::Mutex<BTreeMap<String, PendingAuth>>,
}

impl Gateway {
    /// Desktop-managed runners authenticate with their revocable, runner-scoped
    /// device credential. Operator/admin control tokens are not model keys.
    async fn allows_runner(&self, headers: &HeaderMap, store: &Store) -> bool {
        if self.allows(headers) { return true; }
        let Some(token) = headers.get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "))
            .filter(|v| (32..=100).contains(&v.len())) else { return false; };
        let token = token.to_owned();
        let store = store.clone();
        matches!(tokio::task::spawn_blocking(move || store.authenticate_device(&token)).await,
            Ok(Ok(Some(device))) if device.role == factory_core::DeviceRole::Runner && device.runner_id.is_some())
    }

    pub fn managed(url: String, password: String, key: String) -> Result<Self> {
        let central = Registration { url, password };
        let url = Url::parse(&central.url).context("Invalid central OpenCode URL")?;
        ensure!(
            url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
                && url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none()
                && central.password.len() >= 32,
            "Central OpenCode needs a private loopback registration"
        );
        ensure!(
            (32..=128).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_graphic()),
            "Invalid Factory model key"
        );
        Ok(Self(Arc::new(Inner {
            central,
            keys: vec![Sha256::digest(key).into()],
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .read_timeout(Duration::from_secs(310))
                .build()?,
            upstream: CODEX.into(),
            #[cfg(test)]
            copilot_test_upstream: None,
            pending: Default::default(),
        })))
    }

    fn allows(&self, headers: &HeaderMap) -> bool {
        let Some(token) = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .filter(|v| (32..=128).contains(&v.len()))
        else {
            return false;
        };
        let hash: [u8; 32] = Sha256::digest(token).into();
        self.0
            .keys
            .iter()
            .fold(subtle::Choice::from(0), |ok, key| ok | key.ct_eq(&hash))
            .into()
    }

    async fn entries(&self) -> Option<Vec<Entry>> {
        let mut response = self
            .0
            .client
            .get(format!(
                "{}/api/credential",
                self.0.central.url.trim_end_matches('/')
            ))
            .basic_auth("opencode", Some(&self.0.central.password))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if data.len() + chunk.len() > 1024 * 1024 {
                return None;
            }
            data.extend_from_slice(&chunk);
        }
        let payload: Credentials = serde_json::from_slice(&data).ok()?;
        Some(payload.data)
    }

    async fn credential(&self, id: &str) -> Option<OAuthAccess> {
        let credential = self
            .entries()
            .await?
            .into_iter()
            .find(|entry| entry.id == id && entry.integration_id == "openai")?;
        let access: OAuthAccess = serde_json::from_value(credential.value).ok()?;
        if access.kind != "oauth"
            || access.expires <= crate::unix_ms()
            || access.access.is_empty()
            || access.metadata.account_id.is_empty()
        {
            return None;
        }
        Some(access)
    }

    async fn binding(&self, store: Store, alias: String) -> Option<String> {
        tokio::task::spawn_blocking(move || store.model_binding(&alias))
            .await
            .ok()?
            .ok()?
    }

    pub(super) async fn access_status(&self, store: Store, alias: String) -> &'static str {
        if alias == OPENCODE_FREE_ACCESS {
            return opencode_free::status(self, store).await;
        }
        let provider = factory_protocol::models::KeyModelProvider::from_access_id(&alias);
        let copilot = is_copilot_access(&alias);
        let Some(credential_id) = self.binding(store, alias).await else {
            return "not_connected";
        };
        let ready = match provider {
            Some(provider) => key_provider::credential(self, provider, &credential_id).await.is_some(),
            None if copilot => copilot::credential(self, &credential_id).await.is_some(),
            None => self.credential(&credential_id).await.is_some(),
        };
        if ready {
            "ready"
        } else {
            "unavailable"
        }
    }

    fn oauth(&self, alias: &str, suffix: &str) -> reqwest::RequestBuilder {
        self.0
            .client
            .post(format!(
                "{}/api/integration/{}/connect/oauth{suffix}",
                self.0.central.url.trim_end_matches('/'), oauth_integration(alias)
            ))
            .basic_auth("opencode", Some(&self.0.central.password))
    }
}

fn oauth_integration(alias: &str) -> &'static str {
    if is_copilot_access(alias) { COPILOT_INTEGRATION } else { "openai" }
}

#[derive(Deserialize)]
struct Credentials {
    data: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    id: String,
    #[serde(rename = "integrationID")]
    integration_id: String,
    label: String,
    #[serde(default)]
    active: bool,
    value: Value,
}
#[derive(Deserialize)]
struct OAuthAccess {
    #[serde(rename = "type")]
    kind: String,
    access: String,
    expires: u64,
    metadata: Account,
}
#[derive(Deserialize)]
struct Account {
    #[serde(rename = "accountID")]
    account_id: String,
}

pub(super) fn routes(gateway: Gateway, store: Store) -> Router {
    Router::new()
        .route("/v1/model-gateway/catalog", get(catalog))
        .route("/v1/model-gateway/{access}/responses", post(responses))
        .route("/v1/model-gateway/{access}/chat/completions", post(key_provider::chat))
        .route("/v1/model-gateway/{access}/messages", post(key_provider::messages))
        .route("/v1/model-gateway/{access}/models/{operation}", post(key_provider::gemini))
        .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
        .with_state((gateway, store))
}

fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({
            "error":{"message":message},
            "code":factory_protocol::ErrorCode::InvalidRequest,
            "message":message,
            "request_id":factory_core::RequestId::new()
        })),
    )
        .into_response()
}

async fn catalog(State((gateway, store)): State<(Gateway, Store)>, headers: HeaderMap) -> Response {
    if !gateway.allows_runner(&headers, &store).await {
        return error(StatusCode::UNAUTHORIZED, "Factory model key required");
    }
    let Ok(Ok((settings, bound))) = tokio::task::spawn_blocking(move || {
        Ok::<_, crate::store::StoreError>((store.factory_models()?, store.model_bindings()?))
    })
    .await
    else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Factory model catalog unavailable",
        );
    };
    let accesses: Vec<_> = settings
        .accesses
        .into_iter()
        .filter(|a| a.id == OPENCODE_FREE_ACCESS || bound.contains(&a.id))
        .collect();
    Json(accesses).into_response()
}

async fn responses(
    State((gateway, store)): State<(Gateway, Store)>,
    Path(alias): Path<String>,
    headers: HeaderMap,
    Json(mut data): Json<Value>,
) -> Response {
    if is_copilot_access(&alias) {
        return copilot::relay((gateway, store), alias, headers, data,
            factory_protocol::models::ModelTransport::Responses).await;
    }
    if !gateway.allows_runner(&headers, &store).await {
        return error(StatusCode::UNAUTHORIZED, "Factory model key required");
    }
    let Some(body) = data.as_object_mut() else {
        return error(StatusCode::BAD_REQUEST, "Invalid Responses request");
    };
    let Some(model) = body.get("model").and_then(Value::as_str).map(str::to_owned) else {
        return error(StatusCode::BAD_REQUEST, "Model required");
    };
    let Some(credential_id) = gateway.binding(store.clone(), alias.clone()).await else {
        return error(StatusCode::NOT_FOUND, "Factory model access unavailable");
    };
    let Ok(Ok(settings)) = tokio::task::spawn_blocking(move || store.factory_models()).await else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Factory model catalog unavailable",
        );
    };
    if !settings
        .accesses
        .iter()
        .any(|a| a.id == alias && a.models.contains(&model))
    {
        return error(StatusCode::FORBIDDEN, "Model not permitted for this access");
    }
    if body.contains_key("previous_response_id") || body.contains_key("conversation") {
        return error(
            StatusCode::BAD_REQUEST,
            "Stateful Responses requests are not supported",
        );
    }
    body.insert("store".into(), Value::Bool(false));
    body.insert("stream".into(), Value::Bool(true));
    body.remove("max_output_tokens");
    body.entry("instructions")
        .or_insert_with(|| json!("Follow the instructions in the input."));
    let Some(credential) = gateway.credential(&credential_id).await else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Factory model access not ready",
        );
    };
    let result = gateway
        .0
        .client
        .post(&gateway.0.upstream)
        .bearer_auth(credential.access)
        .header("ChatGPT-Account-Id", credential.metadata.account_id)
        .header("Originator", "opencode")
        .header(header::ACCEPT, "text/event-stream")
        .json(&data)
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => model_stream(response).await,
        Ok(response) => error(
            StatusCode::BAD_GATEWAY,
            &format!(
                "Model provider rejected the request (HTTP {})",
                response.status().as_u16()
            ),
        ),
        Err(_) => error(StatusCode::BAD_GATEWAY, "Model provider unavailable"),
    }
}

async fn model_stream(mut response: reqwest::Response) -> Response {
    use futures_util::StreamExt;
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase());
    let rejected = |format| {
        eprintln!("Model gateway: ChatGPT returned {format} instead of an event stream");
        error(
            StatusCode::BAD_GATEWAY,
            &format!("ChatGPT hat keinen Modell-Stream geliefert ({format})."),
        )
    };
    let body = match content_type.as_deref() {
        Some("text/event-stream") => Body::from_stream(response.bytes_stream()),
        // The ChatGPT Codex endpoint can send SSE without a Content-Type.
        // Inspect only a bounded prefix and replay each chunk unchanged.
        None if !response.headers().contains_key(header::CONTENT_TYPE) => {
            let mut prefix = Vec::new();
            let mut chunks = Vec::new();
            let sniff = async {
                while let Some(chunk) = response.chunk().await? {
                    let limit = (1024 - prefix.len()).min(chunk.len());
                    prefix.extend_from_slice(&chunk[..limit]);
                    chunks.push(chunk);
                    match event_stream_prefix(&prefix) {
                        Some(valid) => return Ok::<_, reqwest::Error>(valid),
                        None if prefix.len() == 1024 => break,
                        None => {}
                    }
                }
                Ok(false)
            };
            match tokio::time::timeout(Duration::from_secs(30), sniff).await {
                Ok(Ok(true)) => Body::from_stream(
                    futures_util::stream::iter(chunks.into_iter().map(Ok::<_, reqwest::Error>))
                        .chain(response.bytes_stream()),
                ),
                Ok(Ok(false)) => return rejected("kein SSE ohne Content-Type"),
                _ => return rejected("Streambeginn nicht verfügbar"),
            }
        }
        Some("text/html") => return rejected("HTML"),
        Some("application/json" | "application/problem+json") => return rejected("JSON"),
        _ => return rejected("anderes Format"),
    };
    ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response()
}

fn event_stream_prefix(bytes: &[u8]) -> Option<bool> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    for line in bytes.split_inclusive(|b| matches!(b, b'\n' | b'\r')) {
        // A first event can be large (for example if it echoes tool schemas).
        // The field name is sufficient; don't wait for the full data line.
        if line.starts_with(b"data:") {
            return Some(true);
        }
        let Some((end, line)) = line.split_last() else {
            continue;
        };
        if !matches!(end, b'\n' | b'\r') {
            return None;
        }
        if line.is_empty() || line.starts_with(b":") {
            continue;
        }
        // `event:`, `id:` and `retry:` are valid metadata, not proof of a payload.
        if ![b"event:".as_slice(), b"id:", b"retry:"]
            .iter()
            .any(|key| line.starts_with(key))
        {
            return Some(false);
        }
    }
    None
}

struct PendingAuth {
    attempt: String,
    label: String,
    expires: u64,
    completing: bool,
    previous: Option<(String, String)>,
    name: Option<String>,
    cancel: Arc<tokio::sync::Notify>,
    auto: bool,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OAuthPrompt {
    #[serde(rename = "attemptID")]
    attempt_id: String,
    url: String,
    instructions: String,
    mode: String,
}

#[derive(Deserialize)]
pub(super) struct FinishAuth {
    attempt_id: String,
    code: Option<String>,
}

fn private_json(body: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

fn login_context(
    app: super::App,
    principal: super::Principal,
    gateway: Option<axum::Extension<Gateway>>,
) -> Result<(Gateway, Store), Response> {
    if principal.role != factory_core::DeviceRole::Admin {
        return Err(error(
            StatusCode::FORBIDDEN,
            "Nur Factory-Admins können Modellzugänge verbinden",
        ));
    }
    gateway
        .map(|axum::Extension(gateway)| (gateway, app.store))
        .ok_or_else(|| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Factory model gateway not configured",
            )
        })
}

pub(super) async fn remove_access(
    State(app): State<super::App>,
    axum::Extension(principal): axum::Extension<super::Principal>,
    gateway: Option<axum::Extension<Gateway>>,
    Path(alias): Path<String>,
) -> Response {
    let (gateway, store) = match login_context(app, principal, gateway) {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    // Complete local revocation and credential cleanup even if the UI closes.
    tokio::spawn(async move {
        let pending = gateway.0.pending.lock().await;
        if pending.get(&alias).is_some_and(|p| p.completing || p.expires > crate::unix_ms()) {
            return error(StatusCode::CONFLICT, "Die Anmeldung für diesen Modellzugang läuft noch.");
        }
        let (settings, credential) = match tokio::task::spawn_blocking(move || store.remove_model_access(&alias)).await {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => return crate::db_error(err, factory_core::RequestId::new()),
            Err(_) => return crate::db_error(crate::store::StoreError::Internal, factory_core::RequestId::new()),
        };
        drop(pending);
        let mut credential_removed = true;
        if let Some(credential) = credential {
            let mut url = Url::parse(&gateway.0.central.url).expect("validated central URL");
            url.path_segments_mut().expect("validated central URL")
                .pop_if_empty().extend(["api", "credential", &credential]);
            credential_removed = gateway.0.client.delete(url)
                .basic_auth("opencode", Some(&gateway.0.central.password))
                .timeout(Duration::from_secs(5))
                .send().await.is_ok_and(|r| r.status().is_success() || r.status() == StatusCode::NOT_FOUND);
        }
        private_json(json!({"settings":settings,"credential_removed":credential_removed}))
    }).await.unwrap_or_else(|_| {
        crate::db_error(crate::store::StoreError::Internal, factory_core::RequestId::new())
    })
}

pub(super) async fn begin(
    State(app): State<super::App>,
    axum::Extension(principal): axum::Extension<super::Principal>,
    gateway: Option<axum::Extension<Gateway>>,
    Path((alias, provider)): Path<(String, String)>,
) -> Response {
    if provider != oauth_integration(&alias) {
        return error(StatusCode::NOT_FOUND, "Unknown OAuth provider");
    }
    let (gateway, store) = match login_context(app, principal, gateway) {
        Ok(ctx) => ctx,
        Err(e) => return e,
    };
    begin_attempt(gateway, store, alias, None).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NameAccess {
    name: String,
}

pub(super) async fn begin_named(
    State(app): State<super::App>,
    axum::Extension(principal): axum::Extension<super::Principal>,
    gateway: Option<axum::Extension<Gateway>>,
    Path((alias, provider)): Path<(String, String)>,
    Json(body): Json<NameAccess>,
) -> Response {
    if provider != oauth_integration(&alias) {
        return error(StatusCode::NOT_FOUND, "Unknown OAuth provider");
    }
    let (gateway, store) = match login_context(app, principal, gateway) {
        Ok(ctx) => ctx,
        Err(e) => return e,
    };
    let name = body.name.trim();
    if !crate::valid_text(name, 120)
        || !(is_copilot_access(&alias) || alias
            .strip_prefix("factory-openai-")
            .is_some_and(|id| id.parse::<factory_core::RequestId>().is_ok()))
    {
        return error(
            StatusCode::BAD_REQUEST,
            "Bitte einen gültigen Zugangsnamen angeben.",
        );
    }
    begin_attempt(gateway, store, alias, Some(name.into())).await
}

async fn begin_attempt(
    gateway: Gateway,
    store: Store,
    alias: String,
    name: Option<String>,
) -> Response {
    if factory_protocol::models::KeyModelProvider::from_access_id(&alias).is_some() {
        return error(StatusCode::BAD_REQUEST, "Use an API key for this provider");
    }
    let catalog = store.clone();
    let Ok(Ok(settings)) = tokio::task::spawn_blocking(move || catalog.factory_models()).await
    else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Factory model catalog unavailable",
        );
    };
    if let Some(name) = &name {
        if settings.accesses.len() >= 64
            || settings
                .accesses
                .iter()
                .any(|a| a.id == alias || a.name.trim().to_lowercase() == name.to_lowercase())
        {
            return error(
                StatusCode::CONFLICT,
                "Der Name ist bereits vergeben oder das Zugangslimit ist erreicht.",
            );
        }
    } else if !settings.accesses.iter().any(|a| a.id == alias) {
        return error(
            StatusCode::NOT_FOUND,
            "Create the named Factory model access first",
        );
    }
    let previous = if let Some(id) = gateway.binding(store.clone(), alias.clone()).await {
        let account = match gateway.entries().await.and_then(|entries| entries.into_iter()
            .find(|e| e.id == id && e.integration_id == oauth_integration(&alias))) {
            Some(entry) => oauth_account(&gateway, &entry).await,
            None => None,
        };
        let Some(account) = account else {
            return error(StatusCode::CONFLICT, "Der bisherige Kontoinhaber lässt sich nicht prüfen. Einen neuen benannten Zugang anlegen.");
        };
        Some((id, account))
    } else {
        None
    };
    let mut attempts = gateway.0.pending.lock().await;
    attempts.retain(|_, pending| pending.completing || pending.expires > crate::unix_ms());
    // The upstream model catalog is scoped to the active account. Serialize
    // OAuth, but never switch accounts for inference.
    if !attempts.is_empty() {
        return error(
            StatusCode::CONFLICT,
            "An OAuth attempt for this access is already in progress",
        );
    }
    let copilot = is_copilot_access(&alias);
    let method = if copilot { "device" } else { "chatgpt-headless" };
    if !wait_for_oauth_method(&gateway, oauth_integration(&alias), method).await {
        return error(StatusCode::SERVICE_UNAVAILABLE, "OpenCode bietet diese Anmeldung noch nicht an. Bitte erneut versuchen.");
    }
    let label = format!("{alias}-{}", factory_core::RequestId::new());
    let mut input = json!({"methodID":method,"label":label});
    if copilot {
        input["answer"] = json!({"deploymentType":"github.com"});
    }
    let response = gateway
        .oauth(&alias, "")
        .timeout(Duration::from_secs(10))
        .json(&input)
        .send()
        .await;
    let Ok(response) = response.and_then(reqwest::Response::error_for_status) else {
        return error(
            StatusCode::BAD_GATEWAY,
            if copilot { "Die GitHub-Anmeldung konnte nicht gestartet werden. Bitte erneut versuchen." }
            else { "Die ChatGPT-Anmeldung konnte nicht gestartet werden. Bitte erneut versuchen." },
        );
    };
    #[derive(Deserialize)]
    struct ResultPrompt {
        data: OAuthPrompt,
    }
    let Ok(ResultPrompt { data: prompt }) = response.json().await else {
        return error(
            StatusCode::BAD_GATEWAY,
            "Unexpected OpenCode OAuth response",
        );
    };
    if !prompt
        .attempt_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        || prompt.attempt_id.is_empty()
        || !matches!(prompt.mode.as_str(), "auto" | "code")
        || !Url::parse(&prompt.url)
            .is_ok_and(|url| url.scheme() == "https" && url.username().is_empty()
                && url.password().is_none()
                && url.host_str() == Some(if copilot { "github.com" } else { "auth.openai.com" })
                && (!copilot || url.path() == "/login/device"))
    {
        return error(StatusCode::BAD_GATEWAY, "Unexpected OpenCode OAuth prompt");
    }
    attempts.insert(
        alias,
        PendingAuth {
            attempt: prompt.attempt_id.clone(),
            label,
            expires: crate::unix_ms().saturating_add(15 * 60 * 1000),
            completing: false,
            previous,
            name,
            cancel: Arc::new(tokio::sync::Notify::new()),
            auto: prompt.mode == "auto",
        },
    );
    private_json(json!(prompt))
}

async fn wait_for_oauth_method(gateway: &Gateway, integration: &str, method: &str) -> bool {
    #[derive(Deserialize)]
    struct Method {
        id: Option<String>,
        #[serde(rename = "type")]
        kind: String,
    }
    #[derive(Deserialize)]
    struct Integration {
        methods: Vec<Method>,
    }
    #[derive(Deserialize)]
    struct Reply {
        data: Integration,
    }
    let url = format!(
        "{}/api/integration/{integration}", gateway.0.central.url.trim_end_matches('/')
    );
    let wait = async {
        loop {
            let response = gateway
                .0
                .client
                .get(&url)
                .basic_auth("opencode", Some(&gateway.0.central.password))
                .timeout(Duration::from_secs(2))
                .send()
                .await;
            if let Ok(response) = response.and_then(reqwest::Response::error_for_status) {
                if response.json::<Reply>().await.is_ok_and(|r| {
                    r.data
                        .methods
                        .iter()
                        .any(|m| m.kind == "oauth" && m.id.as_deref() == Some(method))
                }) {
                    return true;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(8), wait)
        .await
        .unwrap_or(false)
}

async fn discovered_models(gateway: &Gateway, credential: &str) -> Option<Vec<String>> {
    #[derive(Deserialize)]
    struct Catalog {
        data: Vec<ProviderModel>,
    }
    #[derive(Deserialize)]
    struct ProviderModel {
        id: String,
        #[serde(rename = "modelID")]
        model_id: Option<String>,
        #[serde(rename = "providerID")]
        provider_id: String,
        #[serde(rename = "type")]
        kind: Option<String>,
        enabled: Option<bool>,
    }
    let active = |entries: Vec<Entry>| {
        entries
            .into_iter()
            .any(|e| e.id == credential && e.integration_id == "openai" && e.active)
    };
    for _ in 0..8 {
        if active(gateway.entries().await?) {
            let response = gateway
                .0
                .client
                .get(format!(
                    "{}/api/model",
                    gateway.0.central.url.trim_end_matches('/')
                ))
                .basic_auth("opencode", Some(&gateway.0.central.password))
                .timeout(Duration::from_secs(4))
                .send()
                .await
                .ok()?;
            let catalog: Catalog = response.error_for_status().ok()?.json().await.ok()?;
            let mut seen = std::collections::HashSet::new();
            let models: Vec<_> = catalog
                .data
                .into_iter()
                .filter(|m| {
                    m.provider_id == "openai"
                        && m.kind.as_deref().is_none_or(|kind| kind == "chat")
                        && m.enabled != Some(false)
                })
                .map(|m| m.model_id.unwrap_or(m.id))
                .filter(|id| crate::valid_text(id, 200) && seen.insert(id.clone()))
                .take(256)
                .collect();
            if !models.is_empty() && active(gateway.entries().await?) {
                return Some(models);
            }
        }
        tokio::time::sleep(Duration::from_millis(350)).await;
    }
    None
}

pub(super) async fn status(
    State(app): State<super::App>,
    gateway: Option<axum::Extension<Gateway>>,
    Path(alias): Path<String>,
) -> Response {
    let Some(axum::Extension(gateway)) = gateway else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Factory model gateway not configured",
        );
    };
    private_json(json!({"state": gateway.access_status(app.store, alias).await}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CancelAuth {
    attempt_id: String,
}

pub(super) async fn cancel(
    State(app): State<super::App>,
    axum::Extension(principal): axum::Extension<super::Principal>,
    gateway: Option<axum::Extension<Gateway>>,
    Path((alias, provider)): Path<(String, String)>,
    Json(body): Json<CancelAuth>,
) -> Response {
    if provider != oauth_integration(&alias) {
        return error(StatusCode::NOT_FOUND, "Unknown OAuth provider");
    }
    let (gateway, _) = match login_context(app, principal, gateway) {
        Ok(ctx) => ctx,
        Err(e) => return e,
    };
    let mut pending = gateway.0.pending.lock().await;
    let Some(attempt) = pending.get(&alias).filter(|p| p.attempt == body.attempt_id) else {
        return private_json(json!({"cancelled":true}));
    };
    attempt.cancel.notify_one();
    let attempt = pending.remove(&alias).expect("validated attempt").attempt;
    drop(pending);
    let _ = gateway
        .0
        .client
        .delete(format!(
            "{}/api/integration/{}/connect/oauth/{attempt}",
            gateway.0.central.url.trim_end_matches('/'), oauth_integration(&alias)
        ))
        .basic_auth("opencode", Some(&gateway.0.central.password))
        .timeout(Duration::from_secs(5))
        .send()
        .await;
    private_json(json!({"cancelled":true}))
}

pub(super) async fn finish(
    State(app): State<super::App>,
    axum::Extension(principal): axum::Extension<super::Principal>,
    gateway: Option<axum::Extension<Gateway>>,
    Path((alias, provider)): Path<(String, String)>,
    Json(body): Json<FinishAuth>,
) -> Response {
    if provider != oauth_integration(&alias) {
        return error(StatusCode::NOT_FOUND, "Unknown OAuth provider");
    }
    let (gateway, store) = match login_context(app, principal, gateway) {
        Ok(ctx) => ctx,
        Err(e) => return e,
    };
    if body.code.as_ref().is_some_and(|code| code.len() > 4096) {
        return error(StatusCode::BAD_REQUEST, "Invalid OAuth code");
    }
    let (label, previous, name, cancel, auto) = {
        let mut attempts = gateway.0.pending.lock().await;
        let Some(found) = attempts
            .get_mut(&alias)
            .filter(|p| p.attempt == body.attempt_id && p.expires > crate::unix_ms())
        else {
            return error(
                StatusCode::NOT_FOUND,
                "OAuth attempt unavailable or expired",
            );
        };
        if found.completing {
            return error(StatusCode::CONFLICT, "OAuth completion already in progress");
        }
        found.completing = true;
        (
            found.label.clone(),
            found.previous.clone(),
            found.name.clone(),
            found.cancel.clone(),
            found.auto,
        )
    };
    // Keep the centrally started completion alive even if the desktop request disconnects.
    let copilot = is_copilot_access(&alias);
    let completion = tokio::spawn(async move {
        let wait = async {
            if auto {
                wait_for_oauth_result(&gateway, &alias, &body.attempt_id).await
            } else {
                gateway
                    .oauth(&alias, &format!("/{}/complete", body.attempt_id))
                    .timeout(Duration::from_secs(285))
                    .json(&json!({"code":body.code}))
                    .send()
                    .await
                    .is_ok_and(|r| r.status().is_success())
            }
        };
        let completed = tokio::select! {
            biased;
            _ = cancel.notified() => return None,
            completed = wait => completed,
        };
        let entry = if completed {
            gateway.entries().await.and_then(|entries| {
                entries.into_iter().find(|e| {
                    e.integration_id == oauth_integration(&alias) && e.label == label && e.value["type"] == "oauth"
                })
            })
        } else {
            None
        };
        let mut wrong_account = false;
        let result = if let Some(entry) = entry {
            let id = alias.clone();
            let account = oauth_account(&gateway, &entry).await;
            wrong_account = previous
                .as_ref()
                .is_some_and(|(_, previous_account)| account.as_deref() != Some(previous_account.as_str()));
            if !wrong_account && account.is_some() && name.is_some() {
                let models = tokio::select! {
                    biased;
                    _ = cancel.notified() => return None,
                    models = async {
                        if copilot {
                            copilot::models(&gateway, &entry).await
                        } else {
                            discovered_models(&gateway, &entry.id).await.map(|m| (m, Default::default()))
                        }
                    } => models,
                };
                if let Some((models, transports)) = models {
                    let access = factory_protocol::models::ModelAccess {
                        id,
                        name: name.expect("validated new access"),
                        models,
                        transports,
                    };
                    let pending = gateway.0.pending.lock().await;
                    if !pending
                        .get(&alias)
                        .is_some_and(|p| p.attempt == body.attempt_id)
                    {
                        return None;
                    }
                    let result = tokio::task::spawn_blocking(move || {
                        store.add_connected_model_access(access, &entry.id)
                    })
                    .await
                    .ok()
                    .and_then(Result::ok);
                    drop(pending);
                    result
                } else {
                    None
                }
            } else if !wrong_account && account.is_some() {
                let pending = gateway.0.pending.lock().await;
                if !pending
                    .get(&alias)
                    .is_some_and(|p| p.attempt == body.attempt_id)
                {
                    return None;
                }
                let result = tokio::task::spawn_blocking(move || {
                    store.bind_model_access(
                        &id,
                        &entry.id,
                        previous.as_ref().map(|(id, _)| id.as_str()),
                    )
                })
                .await
                .ok()
                .and_then(Result::ok)
                .map(|()| None);
                drop(pending);
                result
            } else {
                None
            }
        } else {
            None
        };
        let mut pending = gateway.0.pending.lock().await;
        if pending
            .get(&alias)
            .is_some_and(|p| p.attempt == body.attempt_id)
        {
            pending.remove(&alias);
        }
        Some((result, wrong_account, completed))
    });
    match completion.await {
        Ok(None) => private_json(json!({"cancelled":true})),
        Ok(Some((Some(Some(default)), _, _))) => private_json(json!({"connected":true,"automatic_default":default})),
        Ok(Some((Some(None), _, _))) => private_json(json!({"connected":true})),
        Ok(Some((None, true, _))) => error(StatusCode::CONFLICT, "Das angemeldete Konto passt nicht zu diesem Zugang. Die bisherige Zuordnung bleibt erhalten."),
        Ok(Some((None, false, true))) => error(StatusCode::BAD_GATEWAY, "Die Anmeldung war erfolgreich, aber die Factory konnte den Zugang oder seine Modelle nicht übernehmen."),
        _ => error(
            StatusCode::BAD_GATEWAY,
            if copilot { "GitHub hat die Anmeldung nicht bestätigt. Prüfe, ob dein Konto Zugriff auf Copilot hat." }
            else { "ChatGPT hat die Anmeldung nicht bestätigt. Bitte versuche es erneut." },
        ),
    }
}

async fn oauth_account(gateway: &Gateway, entry: &Entry) -> Option<String> {
    if entry.integration_id == COPILOT_INTEGRATION {
        copilot::account(gateway, entry).await
    } else {
        entry.value["metadata"]["accountID"].as_str().filter(|id| !id.is_empty()).map(str::to_owned)
    }
}

async fn wait_for_oauth_result(gateway: &Gateway, alias: &str, attempt: &str) -> bool {
    // OpenCode starts its callback as soon as it creates an auto attempt.
    // POST /complete would start a second callback on OpenCode 2.4.1.
    let url = format!(
        "{}/api/integration/{}/connect/oauth/{attempt}",
        gateway.0.central.url.trim_end_matches('/'), oauth_integration(alias)
    );
    let wait = async {
        loop {
            let response = gateway
                .0
                .client
                .get(&url)
                .basic_auth("opencode", Some(&gateway.0.central.password))
                .timeout(Duration::from_secs(4))
                .send()
                .await;
            if let Ok(response) = response.and_then(reqwest::Response::error_for_status) {
                if let Ok(value) = response.json::<Value>().await {
                    match value["data"]["status"].as_str() {
                        Some("complete") => return true,
                        Some("pending") => {}
                        _ => return false,
                    }
                } else {
                    return false;
                }
            } else {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(750)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(285), wait)
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_core::{RequestId, Revision};
    use factory_protocol::models::{
        FactoryModelSettings, ModelAccess, RoleModels, SetFactoryModels,
    };

    #[tokio::test]
    async fn opencode_free_access_uses_the_native_catalog_without_a_credential() {
        let (central, central_task) = serve(Router::new().route("/api/model", get(|| async {
            Json(json!({"data":[{"id":"big-pickle","providerID":"opencode","type":"chat","enabled":true,
                "capabilities":{"tools":true},"cost":[{"input":0,"output":0}]}]}))
        }))).await;
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store.enable_model_settings().unwrap();
        let key = "factory-model-gateway-test-key-only";
        let admin = "factory-admin-test-key-at-least-thirty-two";
        let gateway = Gateway::managed(central, "test-password-at-least-thirty-two".into(), key.into()).unwrap();
        let (base, factory_task) = serve(crate::router_with_model_gateway(
            admin, store.clone(), Arc::new(std::sync::atomic::AtomicBool::new(false)), Some(gateway),
        )).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let add = |token| client.post(format!("{base}/v1/model-settings/opencode-free")).bearer_auth(token)
            .header(factory_protocol::VERSION_HEADER, "1").json(&json!({"name":"OpenCode Free"})).send();
        assert_eq!(add(key).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        let added: Value = add(admin).await.unwrap().error_for_status().unwrap().json().await.unwrap();
        assert_eq!(added["automatic_default"], json!({"access_id":"opencode","model_id":"big-pickle"}));
        assert!(store.model_binding(OPENCODE_FREE_ACCESS).unwrap().is_none());
        let accesses: Vec<ModelAccess> = client.get(format!("{base}/v1/model-gateway/catalog"))
            .bearer_auth(key).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
        assert_eq!(accesses[0].id, OPENCODE_FREE_ACCESS);
        assert_eq!(accesses[0].models, ["big-pickle"]);
        let status: Value = client.get(format!("{base}/v1/model-settings/accesses/opencode/status"))
            .bearer_auth(admin).header(factory_protocol::VERSION_HEADER, "1")
            .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
        assert_eq!(status["state"], "ready");
        assert_eq!(store.remove_model_access(OPENCODE_FREE_ACCESS).unwrap().1, None);
        factory_task.abort();
        central_task.abort();
    }

    #[tokio::test]
    async fn copilot_device_login_binds_the_account_and_relays_its_native_protocol() {
        use factory_protocol::models::{ModelTransport, COPILOT_ACCESS_PREFIX};
        let label = Arc::new(tokio::sync::Mutex::new(None::<String>));
        let set_label = label.clone();
        let (mock, upstream_task) = serve(Router::new()
            .route("/api/integration/github-copilot", get(|| async {
                Json(json!({"data":{"methods":[{"id":"device","type":"oauth"}]}}))
            }))
            .route("/api/integration/github-copilot/connect/oauth", post(move |Json(body): Json<Value>| async move {
                assert_eq!(body["methodID"], "device");
                assert_eq!(body["answer"], json!({"deploymentType":"github.com"}));
                *set_label.lock().await = body["label"].as_str().map(str::to_owned);
                Json(json!({"data":{"attemptID":"copilot-attempt","mode":"auto",
                    "url":"https://github.com/login/device","instructions":"Enter code: TEST-0000"}}))
            }))
            .route("/api/integration/github-copilot/connect/oauth/copilot-attempt", get(|| async {
                Json(json!({"data":{"status":"complete"}}))
            }))
            .route("/api/credential", get(move || {
                let label = label.clone();
                async move {
                    let label = label.lock().await;
                    Json(json!({"data":label.iter().map(|label| json!({
                        "id":"copilot-credential","label":label,"integrationID":"github-copilot","active":true,
                        "value":{"type":"oauth","methodID":"device","refresh":"only-on-factory","expires":0}
                    })).collect::<Vec<_>>()}))
                }
            }))
            .route("/user", get(|headers: HeaderMap| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer only-on-factory");
                Json(json!({"id":42}))
            }))
            .route("/models", get(|headers: HeaderMap| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer only-on-factory");
                Json(json!({"data":[{
                    "id":"gpt-5","supported_endpoints":["/responses"],
                    "capabilities":{"supports":{"tool_calls":true},"limits":{"max_prompt_tokens":8192,"max_output_tokens":4096}}
                }]}))
            }))
            .route("/responses", post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer only-on-factory");
                assert_eq!(body["store"], false);
                ([(header::CONTENT_TYPE, "text/event-stream")], "data: copilot\n\n")
            }))
            .route("/api/credential/copilot-credential", axum::routing::delete(|| async {
                StatusCode::NO_CONTENT
            }))
        ).await;
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store.enable_model_settings().unwrap();
        let key = "factory-model-gateway-test-key-only";
        let admin = "factory-admin-test-key-at-least-thirty-two";
        let mut gateway = Gateway::managed(mock.clone(), "test-password-at-least-thirty-two".into(), key.into()).unwrap();
        Arc::get_mut(&mut gateway.0).unwrap().copilot_test_upstream = Some(mock);
        let (base, factory_task) = serve(crate::router_with_model_gateway(
            admin, store.clone(), Arc::new(std::sync::atomic::AtomicBool::new(false)), Some(gateway),
        )).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let alias = format!("{COPILOT_ACCESS_PREFIX}{}", RequestId::new());
        let oauth = format!("{base}/v1/model-settings/accesses/{alias}/github-copilot/oauth");
        let start = client.post(format!("{oauth}/start")).bearer_auth(admin)
            .header(factory_protocol::VERSION_HEADER, "1").json(&json!({"name":"Copilot test"}))
            .send().await.unwrap().error_for_status().unwrap().json::<Value>().await.unwrap();
        assert_eq!(start["url"], "https://github.com/login/device");
        let done = client.post(format!("{oauth}/complete")).bearer_auth(admin)
            .header(factory_protocol::VERSION_HEADER, "1").json(&json!({"attempt_id":start["attemptID"]}))
            .send().await.unwrap().error_for_status().unwrap().json::<Value>().await.unwrap();
        assert_eq!(done["connected"], true);
        assert_eq!(store.factory_models().unwrap().accesses[0].transports["gpt-5"], ModelTransport::Responses);
        let infer = |path| client.post(format!("{base}/v1/model-gateway/{alias}/{path}"))
            .bearer_auth(key).json(&json!({"model":"gpt-5","input":[],"store":true})).send();
        assert_eq!(infer("chat/completions").await.unwrap().status(), StatusCode::FORBIDDEN);
        assert_eq!(infer("responses").await.unwrap().text().await.unwrap(), "data: copilot\n\n");
        let remove = |token| client.delete(format!("{base}/v1/model-settings/accesses/{alias}"))
            .bearer_auth(token).header(factory_protocol::VERSION_HEADER, "1").send();
        assert_eq!(remove(key).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        let removed = remove(admin).await.unwrap().error_for_status().unwrap().json::<Value>().await.unwrap();
        assert_eq!(removed["credential_removed"], true);
        assert!(store.model_binding(&alias).unwrap().is_none());
        let settings = store.factory_models().unwrap();
        assert!(settings.accesses.is_empty() && settings.default_model.is_none());
        assert_eq!(infer("responses").await.unwrap().status(), StatusCode::FORBIDDEN);
        factory_task.abort();
        upstream_task.abort();
    }

    #[tokio::test]
    async fn runner_model_access_is_revocable_and_does_not_accept_control_credentials() {
        use factory_core::{DeviceRole, RunnerId};
        use factory_protocol::{CreateEnrollment, RedeemEnrollment};
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let gateway = Gateway::managed("http://127.0.0.1:1".into(), "p".repeat(40), "k".repeat(40)).unwrap();
        let headers = |token: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
            headers
        };
        assert!(gateway.allows_runner(&headers(&"k".repeat(40)), &store).await);
        assert!(!gateway.allows_runner(&headers(&"x".repeat(40)), &store).await);
        for role in [DeviceRole::Admin, DeviceRole::Operator, DeviceRole::Runner] {
            let invitation = store.create_enrollment(CreateEnrollment {
                role, runner_id: (role == DeviceRole::Runner).then(RunnerId::new), ttl_seconds: 60,
            }).unwrap();
            let credential = store.redeem_enrollment(RedeemEnrollment {
                code: invitation.code, name: "Test".into(),
            }).unwrap();
            assert_eq!(gateway.allows_runner(&headers(&credential.token), &store).await, role == DeviceRole::Runner);
            store.revoke_device(credential.device.id).unwrap();
            assert!(!gateway.allows_runner(&headers(&credential.token), &store).await);
        }
    }

    async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (url, task)
    }

    #[tokio::test]
    async fn missing_content_type_only_allows_sse_and_preserves_the_stream() {
        assert_eq!(
            event_stream_prefix(b"event: response.created\ndata: {"),
            Some(true)
        );
        let mock = Router::new()
            .route(
                "/stream",
                get(|| async {
                    let chunks = futures_util::stream::iter([
                        Ok::<_, std::io::Error>(": hello\n\nevent: response.created\n"),
                        Ok("da"),
                        Ok("ta: {\"type\":\"response.created\"}\n\n"),
                        Ok("data: [DONE]\n\n"),
                    ]);
                    Body::from_stream(chunks)
                }),
            )
            .route(
                "/json",
                get(|| async { Body::from("{\"error\":\"not a stream\"}\n") }),
            )
            .route("/empty", get(|| async { Body::empty() }));
        let (url, server) = serve(mock).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        for suffix in ["/stream", "/json", "/empty"] {
            let response = http.get(format!("{url}{suffix}")).send().await.unwrap();
            assert!(!response.headers().contains_key(header::CONTENT_TYPE));
            let response = model_stream(response).await;
            if suffix == "/stream" {
                assert_eq!(
                    response.headers()[header::CONTENT_TYPE],
                    "text/event-stream"
                );
                let bytes = axum::body::to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap();
                assert_eq!(bytes, ": hello\n\nevent: response.created\ndata: {\"type\":\"response.created\"}\n\ndata: [DONE]\n\n");
            } else {
                assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
            }
        }
        server.abort();
    }

    #[derive(Default)]
    struct LoginMock {
        labels: BTreeMap<String, String>,
        credentials: Vec<Value>,
        discovery_reads: usize,
        status_reads: usize,
    }
    type Login = Arc<tokio::sync::Mutex<LoginMock>>;

    #[tokio::test]
    async fn factory_auth_is_central_and_independent_accesses_stream_without_account_switching() {
        let abort_alias = format!("factory-openai-{}", RequestId::new());
        let abort_prefix = abort_alias.clone();
        let waiting = Arc::new(tokio::sync::Notify::new());
        let waiting_mock = waiting.clone();
        let fake = Router::new()
            .route("/api/integration/openai", get(|State(state): State<Login>| async move {
                let mut state = state.lock().await;
                state.discovery_reads += 1;
                let methods = if state.discovery_reads == 1 { json!([]) } else {
                    json!([{"id":"chatgpt-headless","type":"oauth"}])
                };
                Json(json!({"data":{"methods":methods}}))
            }))
            .route("/api/integration/openai/connect/oauth", post(
                |State(state): State<Login>, Json(body): Json<Value>| async move {
                    let mut state = state.lock().await;
                    assert!(state.discovery_reads >= 2, "OAuth was attempted before plugin readiness");
                    let label = body["label"].as_str().unwrap();
                    assert_eq!(body["methodID"], "chatgpt-headless");
                    let id = (state.labels.len() + 1).to_string();
                    state.labels.insert(id.clone(), label.into());
                    Json(json!({"data":{"attemptID":id,"url":"https://auth.openai.com/device","instructions":"Demo","mode":"auto"}}))
                }))
            .route("/api/integration/openai/connect/oauth/{attempt}", get(
                move |State(state): State<Login>, Path(id): Path<String>| {
                    let waiting_mock = waiting_mock.clone();
                    let abort_prefix = abort_prefix.clone();
                    async move {
                    let mut state = state.lock().await;
                    let label = state.labels[&id].clone();
                    if label.starts_with(&abort_prefix) {
                        drop(state);
                        waiting_mock.notify_one();
                        return Json(json!({"data":{"status":"pending"}}));
                    }
                    state.status_reads += 1;
                    if state.status_reads == 1 {
                        return Json(json!({"data":{"status":"pending"}}));
                    }
                    for entry in &mut state.credentials { entry["active"] = json!(false); }
                    let account = if id == "4" { "1" } else { id.as_str() };
                    state.credentials.push(json!({"id":id,"label":label,"integrationID":"openai",
                        "active": true, "value":{"type":"oauth", "access":format!("oauth-{id}"),
                            "refresh":format!("refresh-secret-{id}"), "expires": crate::unix_ms() + 60_000,
                            "metadata":{"accountID":account}}}));
                    Json(json!({"data":{"status":"complete"}}))
                }}))
            .route("/api/credential", get(|State(state): State<Login>| async move {
                Json(json!({"data":state.lock().await.credentials}))
            }))
            .route("/api/model", get(|| async {
                Json(json!({"data":[
                    {"id":"example-alias","modelID":"example-model","providerID":"openai","type":"chat","enabled":true},
                    {"id":"image","providerID":"openai","type":"image"},
                    {"id":"no","providerID":"openai","type":"chat","enabled":false},
                    {"id":"other","providerID":"another","type":"chat"},
                ]}))
            }))
            .route("/responses", post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                let id = headers["chatgpt-account-id"].to_str().unwrap();
                assert_eq!(headers[header::AUTHORIZATION], format!("Bearer oauth-{id}"));
                assert_eq!(body["model"], "test-model");
                assert_eq!(body["store"], false);
                assert_eq!(body["stream"], true);
                assert!(!body.as_object().unwrap().contains_key("max_output_tokens"));
                ([(header::CONTENT_TYPE, "text/event-stream")], format!("data: account-{id}\n\n"))
            }))
            .with_state(Login::default());
        let (mock, mock_task) = serve(fake).await;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("factory.sqlite")).unwrap();
        store
            .set_factory_models(SetFactoryModels {
                request_id: RequestId::new(),
                settings: FactoryModelSettings {
                    revision: Revision(0),
                    default_model: None,
                    roles: RoleModels::default(),
                    accesses: ["factory-work", "factory-private"]
                        .into_iter()
                        .map(|name| ModelAccess {
                            id: name.into(),
                            name: name.into(),
                            models: vec!["test-model".into()],
                            transports: Default::default(),
                        })
                        .collect(),
                },
            })
            .unwrap();
        let key1 = "model-key-one-at-least-thirty-two";
        let key2 = "model-key-two-at-least-thirty-two";
        let admin = "factory-admin-at-least-thirty-two";
        let gateway = Gateway(Arc::new(Inner {
            central: Registration {
                url: mock.clone(),
                password: "private".into(),
            },
            keys: [key1, key2]
                .into_iter()
                .map(|key| Sha256::digest(key).into())
                .collect(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            upstream: format!("{mock}/responses"),
            copilot_test_upstream: None,
            pending: Default::default(),
        }));
        let (factory, server) = serve(crate::router_with_model_gateway(
            admin,
            store.clone(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            Some(gateway),
        ))
        .await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let control = |path: &str, bearer: &str| {
            http.post(format!("{factory}{path}"))
                .bearer_auth(bearer)
                .header(factory_protocol::VERSION_HEADER, "1")
        };
        for (alias, id) in [("factory-work", "1"), ("factory-private", "2")] {
            let oath = format!("/v1/model-settings/accesses/{alias}/openai/oauth");
            assert_eq!(
                control(&oath, key1).send().await.unwrap().status(),
                StatusCode::UNAUTHORIZED
            );
            let begun = control(&oath, admin).send().await.unwrap();
            assert_eq!(begun.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(begun.json::<Value>().await.unwrap()["attemptID"], id);
            let completed = control(&format!("{oath}/complete"), admin)
                .json(&json!({"attempt_id":id}))
                .send()
                .await
                .unwrap();
            assert_eq!(
                completed.json::<Value>().await.unwrap(),
                json!({"connected":true})
            );
            assert_eq!(store.model_binding(alias).unwrap().as_deref(), Some(id));
        }
        let infer = |alias: &str, token: &str, model: &str| {
            http.post(format!("{factory}/v1/model-gateway/{alias}/responses"))
                .bearer_auth(token)
                .header("ChatGPT-Account-Id", "ignored")
                .json(&json!({"model":model,"input":[],"max_output_tokens":10,"store":true}))
                .send()
        };
        let (a, b) = tokio::join!(
            infer("factory-work", key1, "test-model"),
            infer("factory-private", key2, "test-model")
        );
        assert_eq!(a.unwrap().text().await.unwrap(), "data: account-1\n\n");
        assert_eq!(b.unwrap().text().await.unwrap(), "data: account-2\n\n");
        assert_eq!(
            infer("factory-work", key2, "test-model")
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "data: account-1\n\n"
        );
        assert_eq!(
            infer("factory-work", admin, "test-model")
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            infer("factory-work", key1, "other").await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let path = "/v1/model-settings/accesses/factory-work/openai/oauth";
        for (attempt, expected, binding) in
            [("3", StatusCode::CONFLICT, "1"), ("4", StatusCode::OK, "4")]
        {
            let begun: Value = control(path, admin)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(begun["attemptID"], attempt);
            let completed = control(&format!("{path}/complete"), admin)
                .json(&json!({"attempt_id":attempt}))
                .send()
                .await
                .unwrap();
            assert_eq!(completed.status(), expected);
            assert_eq!(
                store.model_binding("factory-work").unwrap().as_deref(),
                Some(binding)
            );
        }
        let begun: Value = control(path, admin)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let payload = json!({"attempt_id":begun["attemptID"]});
        let cancelled: Value = control(&format!("{path}/cancel"), admin)
            .json(&payload)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(cancelled["cancelled"], true);
        assert_eq!(
            control(&format!("{path}/complete"), admin)
                .json(&payload)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let alias = format!("factory-openai-{}", RequestId::new());
        let path = format!("/v1/model-settings/accesses/{alias}/openai/oauth");
        let begun: Value = control(&format!("{path}/start"), admin)
            .json(&json!({"name":"Neuer Zugang"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(!store
            .factory_models()
            .unwrap()
            .accesses
            .iter()
            .any(|a| a.id == alias));
        control(&format!("{path}/complete"), admin)
            .json(&json!({"attempt_id":begun["attemptID"]}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let saved = store
            .factory_models()
            .unwrap()
            .accesses
            .into_iter()
            .find(|a| a.id == alias)
            .unwrap();
        assert_eq!(saved.name, "Neuer Zugang");
        assert_eq!(saved.models, ["example-model"]);
        let path = format!("/v1/model-settings/accesses/{abort_alias}/openai/oauth");
        let begun: Value = control(&format!("{path}/start"), admin)
            .json(&json!({"name":"Wird abgebrochen"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let payload = json!({"attempt_id":begun["attemptID"]});
        let completion = tokio::spawn(
            control(&format!("{path}/complete"), admin)
                .json(&payload)
                .send(),
        );
        tokio::time::timeout(Duration::from_secs(2), waiting.notified())
            .await
            .unwrap();
        let cancelled: Value = control(&format!("{path}/cancel"), admin)
            .json(&payload)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(cancelled["cancelled"], true);
        let completion: Value = tokio::time::timeout(Duration::from_secs(2), completion)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(completion["cancelled"], true);
        assert!(store.model_binding(&abort_alias).unwrap().is_none());
        server.abort();
        mock_task.abort();
    }
}
