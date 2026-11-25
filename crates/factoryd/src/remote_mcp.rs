//! Scoped Streamable HTTP relay. Runner tokens never carry upstream credentials.
pub(super) mod discovery;
pub(super) mod oauth;
use super::*;
use axum::{
    body::{Body, Bytes},
    http::{header, HeaderValue, Method},
    Extension,
};
use factory_protocol::mcp::{
    RemoteMcp, RemoteMcpAuth, RemoteMcpLease, RemoveRemoteMcp, SaveRemoteMcp, REMOTE_MCP_CAPABILITY,
};
use ring::{
    aead, hmac,
    rand::{SecureRandom, SystemRandom},
};
use std::{
    fs,
    io::Write,
    net::{IpAddr, SocketAddr},
    os::unix::fs::PermissionsExt,
    path::Path as FsPath,
    time::Duration,
};

const SESSION: &str = "mcp-session-id";
const PROTOCOL: &str = "mcp-protocol-version";
const PROXY_PATH: &str = "/mcp-runtime/";

#[derive(Clone)]
pub struct Gateway(Arc<Keys>);

struct Keys {
    vault: aead::LessSafeKey,
    lease: hmac::Key,
    oauth_pending: tokio::sync::Mutex<BTreeMap<String, oauth::Attempt>>,
    oauth_results: tokio::sync::Mutex<BTreeMap<String, oauth::Outcome>>,
    oauth_refresh: tokio::sync::Mutex<()>,
    oauth_origin: Option<url::Url>,
}

impl Gateway {
    pub fn open(path: &FsPath, must_exist: bool) -> anyhow::Result<Self> {
        let rng = SystemRandom::new();
        if fs::symlink_metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
            anyhow::ensure!(!must_exist, "Remote MCP vault key is missing");
            let mut key = [0; 32];
            rng.fill(&mut key)
                .map_err(|_| anyhow::anyhow!("MCP key generation failed"))?;
            let mut temp = tempfile::NamedTempFile::new_in(
                path.parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(FsPath::new(".")),
            )?;
            temp.write_all(&key)?;
            temp.as_file().sync_all()?;
            match temp.persist_noclobber(path) {
                Ok(_) => {}
                Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.error.into()),
            }
            fs::File::open(
                path.parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(FsPath::new(".")),
            )?
            .sync_all()?;
        }
        let meta = fs::symlink_metadata(path)?;
        anyhow::ensure!(
            meta.is_file() && meta.permissions().mode() & 0o077 == 0 && meta.len() == 32,
            "Remote MCP vault key must be a private regular file of 32 bytes"
        );
        let key = fs::read(path)?;
        let vault = aead::UnboundKey::new(&aead::AES_256_GCM, &key)
            .map_err(|_| anyhow::anyhow!("Invalid MCP vault key"))?;
        let lease = hmac::Key::generate(hmac::HMAC_SHA256, &rng)
            .map_err(|_| anyhow::anyhow!("MCP lease key generation failed"))?;
        Ok(Self(Arc::new(Keys {
            vault: aead::LessSafeKey::new(vault),
            lease,
            oauth_pending: Default::default(),
            oauth_results: Default::default(),
            oauth_refresh: Default::default(),
            oauth_origin: None,
        })))
    }

    pub fn with_oauth_origin(mut self, origin: &str) -> anyhow::Result<Self> {
        let url = valid_url(origin)
            .filter(|url| url.path() == "/")
            .ok_or_else(|| anyhow::anyhow!("FACTORY_MCP_PUBLIC_URL must be an HTTPS origin"))?;
        Arc::get_mut(&mut self.0)
            .ok_or_else(|| anyhow::anyhow!("Configure the MCP gateway before starting"))?
            .oauth_origin = Some(url);
        Ok(self)
    }

    fn seal(&self, id: &str, mut secret: Vec<u8>) -> Option<Vec<u8>> {
        let mut nonce = [0; 12];
        SystemRandom::new().fill(&mut nonce).ok()?;
        self.0
            .vault
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(id.as_bytes()),
                &mut secret,
            )
            .ok()?;
        Some([nonce.as_slice(), &secret].concat())
    }

    fn decrypt(&self, id: &str, mut encrypted: Vec<u8>) -> Option<Vec<u8>> {
        let nonce: [u8; 12] = encrypted.get(..12)?.try_into().ok()?;
        let plain = self
            .0
            .vault
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(id.as_bytes()),
                &mut encrypted[12..],
            )
            .ok()?;
        Some(plain.to_vec())
    }

    fn open_secret(&self, id: &str, encrypted: Vec<u8>) -> Option<HeaderValue> {
        let mut value = HeaderValue::from_bytes(&self.decrypt(id, encrypted)?).ok()?;
        value.set_sensitive(true);
        Some(value)
    }

    fn token(&self, path: &str, registration: RegistrationId) -> String {
        hmac::sign(&self.0.lease, format!("{registration}:{path}").as_bytes())
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

fn only_admin(principal: &Principal) -> bool {
    principal.role == DeviceRole::Admin
}

fn missing() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Internal,
        "Remote-MCP-Verwaltung ist nicht verfügbar",
    )
}

fn denied() -> Response {
    StatusCode::FORBIDDEN.into_response()
}

fn bad() -> Response {
    error(
        StatusCode::BAD_REQUEST,
        ErrorCode::InvalidRequest,
        "Ungültiger Remote-MCP, Header oder Schlüssel",
    )
}

fn reserved(name: &str) -> bool {
    matches!(
        name,
        "host"
            | "connection"
            | "content-type"
            | "content-length"
            | "transfer-encoding"
            | "cookie"
            | "accept"
            | "last-event-id"
            | "te"
            | "trailer"
            | "upgrade"
            | "expect"
    ) || name.starts_with("proxy-")
        || name.starts_with("mcp-")
}

fn permitted_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_multicast()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && ip.octets()[0] != 0
                && ip.octets() != [100, 100, 100, 200]
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4() {
                return permitted_ip(mapped.into());
            }
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_unicast_link_local()
                && !ip.is_multicast()
        }
    }
}

fn valid_url(raw: &str) -> Option<url::Url> {
    let url = url::Url::parse(raw).ok()?;
    (raw.len() <= 1024
        && url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.query().is_none()
        && match url.host()? {
            url::Host::Domain(name) => {
                !name.trim_end_matches('.').eq_ignore_ascii_case("localhost")
            }
            url::Host::Ipv4(ip) => permitted_ip(ip.into()),
            url::Host::Ipv6(ip) => permitted_ip(ip.into()),
        })
    .then_some(url)
}

pub(super) async fn list(
    State(app): State<App>,
    Extension(principal): Extension<Principal>,
) -> Response {
    if !only_admin(&principal) {
        return denied();
    }
    db_call(RequestId::new(), move || app.store.remote_mcps()).await
}

pub(super) async fn remove(
    State(app): State<App>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Result<Json<RemoveRemoteMcp>, JsonRejection>,
) -> Response {
    if !only_admin(&principal) {
        return denied();
    }
    let Ok(Json(body)) = body else {
        return bad();
    };
    db_call(body.request_id, move || {
        app.store.remove_remote_mcp(&id, body)
    })
    .await
}

pub(super) async fn save(
    State(app): State<App>,
    Extension(principal): Extension<Principal>,
    gateway: Option<Extension<Gateway>>,
    Path(id): Path<String>,
    body: Result<Json<SaveRemoteMcp>, JsonRejection>,
) -> Response {
    if !only_admin(&principal) {
        return denied();
    }
    let Some(Extension(gateway)) = gateway else {
        return missing();
    };
    let Ok(Json(mut body)) = body else {
        return bad();
    };
    let Some(url) = valid_url(&body.url) else {
        return bad();
    };
    if !valid_repo_skill_id(&id)
        || !valid_text(&body.name, 80)
        || (body.remove_key && body.key.is_some())
    {
        return bad();
    }
    let replacement = match &mut body.auth {
        RemoteMcpAuth::Header { name, bearer } => {
            name.make_ascii_lowercase();
            if name.len() > 80
                || reserved(name)
                || header::HeaderName::from_bytes(name.as_bytes()).is_err()
                || (*bearer && name != "authorization")
            {
                return bad();
            }
            if let Some(key) = body.key.take() {
                if key.len() > 4096 || key.is_empty() || !key.bytes().all(|c| c.is_ascii_graphic())
                {
                    return bad();
                }
                let value = if *bearer {
                    format!("Bearer {key}").into_bytes()
                } else {
                    key.into_bytes()
                };
                let Some(sealed) = gateway.seal(&id, value) else {
                    return missing();
                };
                Some(sealed)
            } else {
                None
            }
        }
        RemoteMcpAuth::None if body.key.is_some() => return bad(),
        RemoteMcpAuth::None => None,
        auth @ RemoteMcpAuth::OAuth { .. } => {
            if body.key.is_some() || !oauth::valid_settings(auth) {
                return bad();
            }
            None
        }
    };
    let meta = RemoteMcp {
        reference: RemoteMcpRef {
            id,
            name: body.name,
            revision: Revision(0),
        },
        url: url.to_string(),
        auth: body.auth,
        has_key: false,
    };
    db_call(RequestId::new(), move || {
        app.store
            .save_remote_mcp(meta, body.expected_revision, replacement, body.remove_key)
    })
    .await
}

fn path_for(runner: RunnerId, command: CommandId, id: &str) -> String {
    format!("{PROXY_PATH}{runner}/{command}/{id}")
}

pub(super) async fn lease(
    State(app): State<App>,
    gateway: Option<Extension<Gateway>>,
    Path((runner, command, id)): Path<(RunnerId, CommandId, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(Extension(gateway)) = gateway else {
        return missing();
    };
    if !valid_repo_skill_id(&id) {
        return bad();
    }
    let guard = match runner_access(&app, runner, &headers).await {
        Ok(guard) => guard,
        Err(error) => return *error,
    };
    let entry = &guard.entries[&runner];
    if !entry
        .input
        .capabilities
        .iter()
        .any(|c| c == REMOTE_MCP_CAPABILITY)
    {
        return denied();
    }
    let registration = entry.registration;
    drop(guard);
    let path = path_for(runner, command, &id);
    if !tokio::task::spawn_blocking(move || app.store.remote_for_command(runner, command, &id))
        .await
        .is_ok_and(|result| result.is_ok())
    {
        return denied();
    }
    let token = gateway.token(&path, registration);
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(RemoteMcpLease { path, token }),
    )
        .into_response()
}

async fn upstream(url: &url::Url) -> Option<reqwest::Client> {
    upstream_with_policy(url, false).await
}

async fn upstream_with_policy(url: &url::Url, public_only: bool) -> Option<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(310));
    let allowed = |ip| permitted_ip(ip) && (!public_only || discovery::public_address(ip));
    if let url::Host::Domain(host) = url.host()? {
        let port = url.port_or_known_default()?;
        let addrs: Vec<SocketAddr> = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .ok()?
        .ok()?
        .take(32)
        .collect();
        if addrs.is_empty() || addrs.iter().any(|a| !allowed(a.ip())) {
            return None;
        }
        builder = builder.resolve_to_addrs(host, &addrs);
    } else if !url
        .host_str()?
        .trim_matches(['[', ']'])
        .parse()
        .is_ok_and(allowed)
    {
        return None;
    }
    builder.build().ok()
}

fn upstream_auth(
    gateway: &Gateway,
    request: reqwest::RequestBuilder,
    meta: &RemoteMcp,
    sealed: Option<Vec<u8>>,
) -> Option<reqwest::RequestBuilder> {
    match &meta.auth {
        RemoteMcpAuth::None => Some(request),
        RemoteMcpAuth::Header { name, .. } => {
            Some(request.header(name, gateway.open_secret(&meta.reference.id, sealed?)?))
        }
        RemoteMcpAuth::OAuth { .. } => {
            let mut value = HeaderValue::from_bytes(&sealed?).ok()?;
            value.set_sensitive(true);
            Some(request.header(header::AUTHORIZATION, value))
        }
    }
}

pub(super) async fn relay(
    State(app): State<App>,
    gateway: Option<Extension<Gateway>>,
    Path((runner, command, id)): Path<(RunnerId, CommandId, String)>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !matches!(method, Method::GET | Method::POST | Method::DELETE) {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let Some(Extension(gateway)) = gateway else {
        return missing();
    };
    if !valid_repo_skill_id(&id) {
        return denied();
    }
    let path = path_for(runner, command, &id);
    let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|t| t.len() == 64)
    else {
        return denied();
    };
    let active = app
        .registry
        .read()
        .await
        .entries
        .get(&runner)
        .filter(|e| e.seen.elapsed() < UNREACHABLE_AFTER)
        .map(|e| e.registration);
    if !active.is_some_and(|registration| {
        bool::from(
            token
                .as_bytes()
                .ct_eq(gateway.token(&path, registration).as_bytes()),
        )
    }) {
        return denied();
    }
    let store = app.store.clone();
    let check_id = id.clone();
    let Ok(Ok((meta, sealed))) =
        tokio::task::spawn_blocking(move || store.remote_for_command(runner, command, &check_id))
            .await
    else {
        return denied();
    };
    let Some(url) = valid_url(&meta.url) else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let Some(client) = upstream(&url).await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let sealed = if matches!(meta.auth, RemoteMcpAuth::OAuth { .. }) {
        let token = oauth::header(&gateway, &app.store, &meta, sealed).await;
        if token.is_none() {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let current = meta.reference.clone();
        if !tokio::task::spawn_blocking(move || {
            app.store
                .remote_for_command(runner, command, &id)
                .is_ok_and(|(entry, _)| entry.reference == current)
        })
        .await
        .unwrap_or(false)
        {
            return denied();
        }
        token
    } else {
        sealed
    };
    forward(&gateway, &meta, sealed, client, url, method, headers, body).await
}

async fn forward(
    gateway: &Gateway,
    meta: &RemoteMcp,
    sealed: Option<Vec<u8>>,
    client: reqwest::Client,
    url: url::Url,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut request = client.request(method, url);
    for name in [
        header::ACCEPT.as_str(),
        header::CONTENT_TYPE.as_str(),
        SESSION,
        PROTOCOL,
        "last-event-id",
    ] {
        if let Some(value) = headers.get(name).filter(|v| v.as_bytes().len() <= 512) {
            request = request.header(name, value);
        }
    }
    if !body.is_empty() {
        request = request.body(body);
    }
    let Some(request) = upstream_auth(gateway, request, meta, sealed) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(upstream) = request.send().await else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    let status = upstream.status();
    if !status.is_success() {
        return match status {
            StatusCode::UNAUTHORIZED
            | StatusCode::FORBIDDEN
            | StatusCode::NOT_FOUND
            | StatusCode::METHOD_NOT_ALLOWED
            | StatusCode::BAD_REQUEST
            | StatusCode::TOO_MANY_REQUESTS => status.into_response(),
            _ => StatusCode::BAD_GATEWAY.into_response(),
        };
    }
    let mut response = if matches!(status, StatusCode::ACCEPTED | StatusCode::NO_CONTENT) {
        (status, Body::empty()).into_response()
    } else {
        let content = upstream
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !(content.starts_with("application/json") || content.starts_with("text/event-stream")) {
            return StatusCode::BAD_GATEWAY.into_response();
        }
        let copied = upstream.headers().clone();
        let mut response = (status, Body::from_stream(upstream.bytes_stream())).into_response();
        for name in [header::CONTENT_TYPE.as_str(), SESSION, PROTOCOL] {
            if let Some(value) = copied.get(name).filter(|v| v.as_bytes().len() <= 512) {
                response
                    .headers_mut()
                    .insert(header::HeaderName::from_static(name), value.clone());
            }
        }
        response
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    response
}

pub(super) async fn test_connection(
    State(app): State<App>,
    Extension(principal): Extension<Principal>,
    gateway: Option<Extension<Gateway>>,
    Path(id): Path<String>,
) -> Response {
    if !only_admin(&principal) {
        return denied();
    }
    let Some(Extension(gateway)) = gateway else {
        return missing();
    };
    let store = app.store.clone();
    let Ok(Ok((meta, sealed))) =
        tokio::task::spawn_blocking(move || store.remote_mcp_credentials(&id)).await
    else {
        return bad();
    };
    let status = tokio::time::timeout(Duration::from_secs(12), async {
        let sealed = if matches!(meta.auth, RemoteMcpAuth::OAuth { .. }) {
            oauth::header(&gateway, &app.store, &meta, sealed).await
        } else {
            sealed
        };
        test_protocol(&gateway, &meta, sealed).await
    })
    .await
    .unwrap_or("unavailable");
    Json(serde_json::json!({"status":status})).into_response()
}

pub(super) async fn verify_draft(
    State(app): State<App>,
    Extension(principal): Extension<Principal>,
    gateway: Option<Extension<Gateway>>,
    Path(id): Path<String>,
    body: Result<Json<SaveRemoteMcp>, JsonRejection>,
) -> Response {
    if !only_admin(&principal) {
        return denied();
    }
    let (Some(Extension(gateway)), Ok(Json(body))) = (gateway, body) else {
        return bad();
    };
    if !valid_repo_skill_id(&id) || body.remove_key {
        return bad();
    }
    let Some(url) = valid_url(&body.url) else {
        return bad();
    };
    let mut meta = RemoteMcp {
        reference: RemoteMcpRef {
            id,
            name: body.name,
            revision: body.expected_revision,
        },
        url: url.to_string(),
        auth: body.auth,
        has_key: false,
    };
    let sealed = match &mut meta.auth {
        RemoteMcpAuth::None if body.key.is_none() => None,
        RemoteMcpAuth::Header { name, bearer } => {
            name.make_ascii_lowercase();
            if name.len() > 80
                || reserved(name)
                || header::HeaderName::from_bytes(name.as_bytes()).is_err()
                || (*bearer && name != "authorization")
            {
                return bad();
            }
            if let Some(key) = body.key {
                if key.is_empty() || key.len() > 4096 || !key.bytes().all(|b| b.is_ascii_graphic())
                {
                    return bad();
                }
                let key = if *bearer {
                    format!("Bearer {key}")
                } else {
                    key
                };
                gateway.seal(&meta.reference.id, key.into_bytes())
            } else {
                let expected = meta.clone();
                tokio::task::spawn_blocking(move || {
                    let (saved, sealed) = app
                        .store
                        .remote_mcp_credentials(&expected.reference.id)
                        .ok()?;
                    (saved.reference.revision == expected.reference.revision
                        && saved.url == expected.url
                        && saved.auth == expected.auth)
                        .then_some(sealed)?
                })
                .await
                .ok()
                .flatten()
            }
        }
        _ => return bad(),
    };
    let status = tokio::time::timeout(
        Duration::from_secs(12),
        test_protocol(&gateway, &meta, sealed),
    )
    .await
    .unwrap_or("unavailable");
    Json(serde_json::json!({"status":status})).into_response()
}

async fn test_protocol(
    gateway: &Gateway,
    meta: &RemoteMcp,
    sealed: Option<Vec<u8>>,
) -> &'static str {
    let Some(url) = valid_url(&meta.url) else {
        return "unavailable";
    };
    let Some(client) = upstream(&url).await else {
        return "unavailable";
    };
    probe_endpoint(gateway, meta, sealed, url, client).await
}

async fn probe_endpoint(
    gateway: &Gateway,
    meta: &RemoteMcp,
    sealed: Option<Vec<u8>>,
    url: url::Url,
    client: reqwest::Client,
) -> &'static str {
    let post = |payload: serde_json::Value| {
        upstream_auth(
            gateway,
            client
                .post(url.clone())
                .header(header::ACCEPT, "application/json, text/event-stream")
                .json(&payload),
            meta,
            sealed.clone(),
        )
    };
    let Some(request) = post(initialize_request()) else {
        return "needs_auth";
    };
    let Ok(response) = request.send().await else {
        return "unavailable";
    };
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        return "needs_auth";
    }
    if !response.status().is_success() {
        return "unavailable";
    }
    let delete = upstream_auth(gateway, client.delete(url.clone()), meta, sealed.clone());
    match finish_probe(response, post, delete).await {
        ProbeResult::Connected => "connected",
        ProbeResult::NeedsAuth(_) => "needs_auth",
        ProbeResult::Unavailable => "unavailable",
    }
}

fn initialize_request() -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"YoloForge","version":"1"}
    }})
}

enum ProbeResult {
    Connected,
    NeedsAuth(HeaderMap),
    Unavailable,
}

async fn finish_probe(
    response: reqwest::Response,
    post: impl Fn(serde_json::Value) -> Option<reqwest::RequestBuilder>,
    delete: Option<reqwest::RequestBuilder>,
) -> ProbeResult {
    let session = response
        .headers()
        .get(SESSION)
        .filter(|v| v.as_bytes().len() <= 512)
        .cloned();
    let Some(reply) = rpc_reply(response, 1).await else {
        return ProbeResult::Unavailable;
    };
    let Some(version) = reply
        .pointer("/result/protocolVersion")
        .and_then(serde_json::Value::as_str)
    else {
        return ProbeResult::Unavailable;
    };
    let apply = |mut request: reqwest::RequestBuilder| {
        request = request.header(PROTOCOL, version);
        if let Some(session) = &session {
            request = request.header(SESSION, session);
        }
        request
    };
    let Some(notification) =
        post(serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
    else {
        return ProbeResult::Unavailable;
    };
    let Ok(notification) = apply(notification).send().await else {
        return ProbeResult::Unavailable;
    };
    if !notification.status().is_success() {
        return probe_failure(&notification);
    }
    let Some(list) = post(serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})) else {
        return ProbeResult::Unavailable;
    };
    let Ok(response) = apply(list).send().await else {
        return ProbeResult::Unavailable;
    };
    let result = if !response.status().is_success() {
        probe_failure(&response)
    } else if rpc_reply(response, 2).await.is_some_and(|r| {
        r.pointer("/result/tools")
            .is_some_and(serde_json::Value::is_array)
    }) {
        ProbeResult::Connected
    } else {
        ProbeResult::Unavailable
    };
    if session.is_some() {
        if let Some(delete) = delete {
            let _ = apply(delete).timeout(Duration::from_secs(1)).send().await;
        }
    }
    result
}

fn probe_failure(response: &reqwest::Response) -> ProbeResult {
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        ProbeResult::NeedsAuth(response.headers().clone())
    } else {
        ProbeResult::Unavailable
    }
}

async fn rpc_reply(mut response: reqwest::Response, id: u64) -> Option<serde_json::Value> {
    let sse = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"));
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len() + chunk.len() > 512 * 1024 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
        if sse {
            // MCP uses standard SSE records; inspect only terminated records.
            let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
            for event in text.split("\n\n").take(text.matches("\n\n").count()) {
                let data = event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:"))
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) {
                    if value["id"].as_u64() == Some(id) {
                        return Some(value);
                    }
                }
            }
        }
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    (value["id"].as_u64() == Some(id)).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "test-secret-value-do-not-echo";

    fn meta() -> RemoteMcp {
        RemoteMcp {
            reference: RemoteMcpRef {
                id: "docs".into(),
                name: "Docs".into(),
                revision: Revision(0),
            },
            url: "https://docs.example.invalid/mcp".into(),
            auth: RemoteMcpAuth::Header {
                name: "x-api-key".into(),
                bearer: false,
            },
            has_key: false,
        }
    }

    #[test]
    fn vault_is_private_authenticated_and_persists_without_reusing_lease_keys() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("key");
        assert!(Gateway::open(&key_file, true).is_err());
        let gateway = Gateway::open(&key_file, false).unwrap();
        let sealed = gateway.seal("docs", KEY.as_bytes().to_vec()).unwrap();
        assert!(!sealed
            .windows(KEY.len())
            .any(|bytes| bytes == KEY.as_bytes()));
        let reopened = Gateway::open(&key_file, true).unwrap();
        assert!(reopened.open_secret("other", sealed.clone()).is_none());
        assert_eq!(
            reopened
                .open_secret("docs", sealed)
                .unwrap()
                .to_str()
                .unwrap(),
            KEY
        );
        let registration = RegistrationId::new();
        assert_ne!(
            gateway.token("/command/one", registration),
            reopened.token("/command/one", registration)
        );
        assert_ne!(
            gateway.token("/command/one", registration),
            gateway.token("/command/two", registration)
        );
        assert_ne!(
            gateway.token("/command/one", registration),
            gateway.token("/command/one", RegistrationId::new())
        );
        for url in [
            "http://example.com/mcp",
            "https://127.0.0.1/mcp",
            "https://169.254.169.254/mcp",
            "https://[::ffff:127.0.0.1]/mcp",
            "https://example.com/mcp?token=secret",
            "https://a:b@example.com/mcp",
        ] {
            assert!(valid_url(url).is_none(), "{url}");
        }
        assert!(valid_url("https://docs.example.com/mcp").is_some());
    }

    #[tokio::test]
    async fn connection_test_lists_tools_without_calling_any() {
        let dir = tempfile::tempdir().unwrap();
        let gateway = Gateway::open(&dir.path().join("key"), false).unwrap();
        let endpoint = Router::new().route("/mcp", post(|headers: HeaderMap, Json(body): Json<serde_json::Value>| async move {
            if headers.get("x-api-key").is_none_or(|v| v != KEY) {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            match body["method"].as_str() {
                Some("initialize") => ([(SESSION, "test-session")], Json(serde_json::json!({
                    "jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-03-26"}
                }))).into_response(),
                Some("notifications/initialized") => {
                    assert_eq!(headers[SESSION], "test-session");
                    StatusCode::ACCEPTED.into_response()
                }
                Some("tools/list") => {
                    assert_eq!(headers[PROTOCOL], "2025-03-26");
                    ([(header::CONTENT_TYPE, "text/event-stream")],
                     "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[]}}\r\n\r\n").into_response()
                }
                _ => panic!("Connection test attempted an unexpected MCP action"),
            }
        }).delete(|headers: HeaderMap| async move {
            assert_eq!(headers[SESSION], "test-session");
            assert_eq!(headers["x-api-key"], KEY);
            StatusCode::NO_CONTENT
        }));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = url::Url::parse(&format!("http://{}/mcp", socket.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(socket, endpoint).await.unwrap() });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let sealed = gateway.seal("docs", KEY.as_bytes().to_vec());
        assert_eq!(
            probe_endpoint(&gateway, &meta(), sealed, url.clone(), client.clone()).await,
            "connected"
        );
        assert_eq!(
            probe_endpoint(&gateway, &meta(), None, url, client).await,
            "needs_auth"
        );
        server.abort();
    }

    #[tokio::test]
    async fn forwarding_replaces_proxy_authorization_and_filters_headers_and_redirects() {
        let dir = tempfile::tempdir().unwrap();
        let gateway = Gateway::open(&dir.path().join("key"), false).unwrap();
        let endpoint = Router::new()
            .route(
                "/mcp",
                post(|headers: HeaderMap, body: Bytes| async move {
                    assert_eq!(headers["x-api-key"], KEY);
                    assert_eq!(headers[SESSION], "session-a");
                    assert!(!headers.contains_key(header::AUTHORIZATION));
                    assert!(!headers.contains_key(header::COOKIE));
                    if body == "redirect" {
                        (
                            StatusCode::TEMPORARY_REDIRECT,
                            [(header::LOCATION, "/never")],
                        )
                            .into_response()
                    } else {
                        (
                            [
                                (header::CONTENT_TYPE, "text/event-stream"),
                                (header::SET_COOKIE, "hidden=1"),
                                (header::HeaderName::from_static(SESSION), "session-a"),
                            ],
                            "data: test\n\n",
                        )
                            .into_response()
                    }
                }),
            )
            .route("/never", post(|| async { StatusCode::OK }));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = url::Url::parse(&format!("http://{}/mcp", socket.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(socket, endpoint).await.unwrap() });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let sealed = gateway.seal("docs", KEY.as_bytes().to_vec());
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer transport-only"),
        );
        headers.insert(header::COOKIE, HeaderValue::from_static("never=send"));
        headers.insert(
            header::HeaderName::from_static(SESSION),
            HeaderValue::from_static("session-a"),
        );
        let response = forward(
            &gateway,
            &meta(),
            sealed.clone(),
            client.clone(),
            url.clone(),
            Method::POST,
            headers.clone(),
            Bytes::from_static(b"normal"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[SESSION], "session-a");
        assert!(!response.headers().contains_key(header::SET_COOKIE));
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap(),
            "data: test\n\n"
        );
        let response = forward(
            &gateway,
            &meta(),
            sealed,
            client,
            url,
            Method::POST,
            headers,
            Bytes::from_static(b"redirect"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert!(!response.headers().contains_key(header::LOCATION));
        server.abort();
    }

    #[tokio::test]
    async fn gui_key_save_does_not_return_the_key_and_rotation_requires_the_revision() {
        let dir = tempfile::tempdir().unwrap();
        let gateway = Gateway::open(&dir.path().join("key"), false).unwrap();
        let store = Store::open(&dir.path().join("db")).unwrap();
        let app = router("fixture-admin-token", store.clone()).layer(Extension(gateway.clone()));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let request = |method: Method, path: &str| {
            client
                .request(method, format!("{base}{path}"))
                .bearer_auth("fixture-admin-token")
                .header(VERSION_HEADER, "1")
        };
        let path = "/v1/mcp/remote/docs";
        let body = |rev, url, key| {
            serde_json::json!({"expected_revision": rev, "name": "Docs",
            "url": url, "auth":{"type":"header","name":"X-API-Key","bearer":false}, "key":key})
        };
        let reply = request(Method::PUT, path)
            .json(&body(0, "https://docs.example.invalid/mcp", Some(KEY)))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(!reply.contains(KEY));
        assert!(serde_json::from_str::<RemoteMcp>(&reply).unwrap().has_key);
        let (_, sealed) = store.remote_mcp_credentials("docs").unwrap();
        assert!(sealed.is_some());
        let choices = request(Method::GET, "/v1/mcp/local/choices")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(
            choices.contains("Docs")
                && !choices.contains("example.invalid")
                && !choices.contains(KEY)
        );
        let response = request(Method::PUT, path)
            .json(&body(1, "https://other.example.invalid/mcp", None))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = request(Method::PUT, path)
            .json(&body(
                0,
                "https://docs.example.invalid/mcp",
                Some("replacement"),
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            request(
                Method::POST,
                &path_for(RunnerId::new(), CommandId::new(), "docs")
            )
            .send()
            .await
            .unwrap()
            .status(),
            StatusCode::FORBIDDEN
        );
        server.abort();
    }
}
