use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use factory_protocol::mcp::MCP_OAUTH_LOOPBACK_REDIRECT;
use serde::{Deserialize, Serialize};

const CALLBACK: &str = "/mcp-oauth/callback";
const ATTEMPT_MS: u64 = 600_000;

pub(super) struct Attempt {
    entry: RemoteMcp,
    operator: Option<DeviceId>,
    verifier: String,
    expires: u64,
    staged: bool,
}

pub(super) struct Outcome {
    id: String,
    operator: Option<DeviceId>,
    expires: u64,
    saved: Option<bool>,
    staged: Option<(RemoteMcp, Vec<u8>)>,
}

#[derive(Deserialize)]
pub(crate) struct Commit {
    attempt: String,
    name: String,
}

#[derive(Deserialize)]
pub(crate) struct Status {
    attempt: String,
}

#[derive(Deserialize)]
pub(crate) struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Tokens {
    access: String,
    refresh: Option<String>,
    expires: u64,
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    token_type: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

pub(super) fn valid_settings(auth: &RemoteMcpAuth) -> bool {
    let RemoteMcpAuth::OAuth {
        client_id,
        authorization_url,
        token_url,
        redirect_uri,
        scope,
    } = auth
    else {
        return false;
    };
    !client_id.is_empty()
        && client_id.len() <= 512
        && client_id.bytes().all(|b| b.is_ascii_graphic())
        && valid_url(authorization_url).is_some()
        && valid_url(token_url).is_some()
        && (redirect_uri == MCP_OAUTH_LOOPBACK_REDIRECT
            || valid_url(redirect_uri).is_some_and(|url| url.path() == CALLBACK))
        && scope.len() <= 512
        && scope.bytes().all(|b| b == b' ' || b.is_ascii_graphic())
}

fn random() -> Option<String> {
    let mut bytes = [0; 32];
    SystemRandom::new().fill(&mut bytes).ok()?;
    Some(URL_SAFE_NO_PAD.encode(bytes))
}

fn authorization(entry: &RemoteMcp, state: &str, verifier: &str) -> Option<url::Url> {
    let RemoteMcpAuth::OAuth {
        client_id,
        authorization_url,
        redirect_uri,
        scope,
        ..
    } = &entry.auth
    else {
        return None;
    };
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = valid_url(authorization_url)?;
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", redirect_uri),
        ("state", state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("resource", &entry.url),
    ]);
    if !scope.is_empty() {
        url.query_pairs_mut().append_pair("scope", scope);
    }
    Some(url)
}

pub(crate) async fn start(
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
    let Ok(Ok((entry, _))) =
        tokio::task::spawn_blocking(move || app.store.remote_mcp_credentials(&id)).await
    else {
        return bad();
    };
    begin(&gateway, principal, entry, false).await
}

pub(crate) async fn prepare(
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
    let Some(url) = valid_url(&body.url) else {
        return bad();
    };
    if !valid_repo_skill_id(&id)
        || !valid_text(&body.name, 80)
        || !valid_settings(&body.auth)
        || body.key.is_some()
        || body.remove_key
    {
        return bad();
    }
    let entry = RemoteMcp {
        reference: RemoteMcpRef {
            id,
            name: body.name,
            revision: body.expected_revision,
        },
        url: url.to_string(),
        auth: body.auth,
        has_key: false,
    };
    begin(&gateway, principal, entry, true).await
}

pub(crate) async fn commit(
    State(app): State<App>,
    Extension(principal): Extension<Principal>,
    gateway: Option<Extension<Gateway>>,
    Path(id): Path<String>,
    body: Result<Json<Commit>, JsonRejection>,
) -> Response {
    if !only_admin(&principal) {
        return denied();
    }
    let (Some(Extension(gateway)), Ok(Json(body))) = (gateway, body) else {
        return bad();
    };
    if !valid_text(&body.name, 80) {
        return bad();
    }
    let mut outcomes = gateway.0.oauth_results.lock().await;
    let Some(outcome) = outcomes.get_mut(&body.attempt).filter(|outcome| {
        outcome.id == id && outcome.operator == principal.device_id && outcome.expires > unix_ms()
    }) else {
        return bad();
    };
    let Some((mut entry, sealed)) = outcome.staged.take() else {
        return bad();
    };
    drop(outcomes);
    entry.reference.name = body.name;
    let revision = entry.reference.revision;
    db_call(RequestId::new(), move || {
        app.store
            .save_remote_mcp(entry, revision, Some(sealed), false)
    })
    .await
}

async fn begin(
    gateway: &Gateway,
    principal: Principal,
    entry: RemoteMcp,
    staged: bool,
) -> Response {
    let (Some(state), Some(verifier)) = (random(), random()) else {
        return missing();
    };
    let Some(url) = authorization(&entry, &state, &verifier) else {
        return bad();
    };
    let now = unix_ms();
    let mut pending = gateway.0.oauth_pending.lock().await;
    pending.retain(|_, a| a.expires > now && a.entry.reference.id != entry.reference.id);
    if pending.len() >= 32 {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    pending.insert(
        state.clone(),
        Attempt {
            entry,
            operator: principal.device_id,
            verifier,
            expires: now + ATTEMPT_MS,
            staged,
        },
    );
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({"url":url.as_str(),"attempt":state})),
    )
        .into_response()
}

pub(crate) async fn status(
    State(app): State<App>,
    Extension(principal): Extension<Principal>,
    gateway: Option<Extension<Gateway>>,
    Path(id): Path<String>,
    body: Result<Json<Status>, JsonRejection>,
) -> Response {
    if !only_admin(&principal) {
        return denied();
    }
    let (Some(Extension(gateway)), Ok(Json(body))) = (gateway, body) else {
        return bad();
    };
    if body.attempt.len() != 43 || !valid_repo_skill_id(&id) {
        return bad();
    }
    let now = unix_ms();
    let pending = gateway.0.oauth_pending.lock().await;
    let outcomes = gateway.0.oauth_results.lock().await;
    let result = if let Some(value) = outcomes.get(&body.attempt).filter(|value| {
        value.id == id && value.operator == principal.device_id && value.expires > now
    }) {
        match value.saved {
            Some(true) if value.staged.is_some() => "verified",
            Some(true) => "connected",
            Some(false) => "failed",
            None => "pending",
        }
    } else if pending.get(&body.attempt).is_some_and(|value| {
        value.entry.reference.id == id
            && value.operator == principal.device_id
            && value.expires > now
    }) {
        "pending"
    } else {
        "failed"
    };
    drop(outcomes);
    drop(pending);
    let entry = if result == "connected" {
        tokio::task::spawn_blocking(move || {
            app.store
                .remote_mcp_credentials(&id)
                .ok()
                .map(|value| value.0)
        })
        .await
        .ok()
        .flatten()
    } else {
        None
    };
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({"status":result,"entry":entry})),
    )
        .into_response()
}

pub(crate) async fn callback(
    State(app): State<App>,
    gateway: Option<Extension<Gateway>>,
    query: Result<Query<Callback>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let (Some(Extension(gateway)), Ok(Query(query))) = (gateway, query) else {
        return page(false);
    };
    page(finish_callback(app, &gateway, query, None).await)
}

pub(crate) async fn complete_loopback(
    State(app): State<App>,
    Extension(principal): Extension<Principal>,
    gateway: Option<Extension<Gateway>>,
    Path(id): Path<String>,
    body: Result<Json<Callback>, JsonRejection>,
) -> Response {
    if !only_admin(&principal) {
        return denied();
    }
    let (Some(Extension(gateway)), Ok(Json(query))) = (gateway, body) else {
        return bad();
    };
    let done = finish_callback(app, &gateway, query, Some((&id, principal.device_id))).await;
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({"completed": done})),
    )
        .into_response()
}

fn accepts_callback(attempt: &Attempt, desktop: Option<(&str, Option<DeviceId>)>) -> bool {
    let RemoteMcpAuth::OAuth { redirect_uri, .. } = &attempt.entry.auth else {
        return false;
    };
    match desktop {
        Some((id, operator)) => {
            redirect_uri == MCP_OAUTH_LOOPBACK_REDIRECT
                && attempt.entry.reference.id == id
                && attempt.operator == operator
        }
        None => redirect_uri != MCP_OAUTH_LOOPBACK_REDIRECT,
    }
}

async fn finish_callback(
    app: App,
    gateway: &Gateway,
    query: Callback,
    desktop: Option<(&str, Option<DeviceId>)>,
) -> bool {
    if query.state.len() != 43
        || !query
            .state
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return false;
    }
    let mut pending = gateway.0.oauth_pending.lock().await;
    if !pending
        .get(&query.state)
        .is_some_and(|attempt| attempt.expires > unix_ms() && accepts_callback(attempt, desktop))
    {
        return false;
    }
    let attempt = pending.remove(&query.state).expect("checked under lock");
    {
        let mut outcomes = gateway.0.oauth_results.lock().await;
        let now = unix_ms();
        outcomes.retain(|_, entry| entry.expires > now);
        if outcomes.len() >= 256 {
            return false;
        }
        outcomes.insert(
            query.state.clone(),
            Outcome {
                id: attempt.entry.reference.id.clone(),
                operator: attempt.operator,
                expires: now + ATTEMPT_MS,
                saved: None,
                staged: None,
            },
        );
    }
    drop(pending);
    let staged = attempt.staged;
    let result = complete(app, gateway, attempt, query.code, query.error.is_some()).await;
    let success = result.is_some();
    if let Some(entry) = gateway.0.oauth_results.lock().await.get_mut(&query.state) {
        entry.saved = Some(success);
        if staged {
            entry.staged = result;
        }
    }
    success
}

async fn complete(
    app: App,
    gateway: &Gateway,
    attempt: Attempt,
    code: Option<String>,
    provider_error: bool,
) -> Option<(RemoteMcp, Vec<u8>)> {
    let Some(code) =
        code.filter(|c| !c.is_empty() && c.len() <= 4096 && !c.chars().any(char::is_control))
    else {
        return None;
    };
    if provider_error {
        return None;
    }
    let principal = Principal {
        role: DeviceRole::Admin,
        device_id: attempt.operator,
        runner_id: None,
    };
    let store = app.store.clone();
    let id = attempt.entry.reference.id.clone();
    let revision = attempt.entry.reference.revision;
    let principal_check = principal.clone();
    let staged = attempt.staged;
    if !tokio::task::spawn_blocking(move || {
        principal_check.active(&store)
            && ((staged && revision == Revision(0))
                || store
                    .remote_mcp_credentials(&id)
                    .is_ok_and(|(current, _)| current.reference.revision == revision))
    })
    .await
    .unwrap_or(false)
    {
        return None;
    }
    let RemoteMcpAuth::OAuth { redirect_uri, .. } = &attempt.entry.auth else {
        return None;
    };
    let Some(tokens) = exchange(
        &attempt.entry,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", &attempt.verifier),
        ],
        None,
    )
    .await
    else {
        eprintln!("MCP OAuth: authorization code could not be exchanged for a token");
        return None;
    };
    let Some(sealed) = encode(gateway, &attempt.entry.reference.id, &tokens) else {
        return None;
    };
    if staged {
        let result = tokio::time::timeout(
            Duration::from_secs(12),
            test_protocol(
                gateway,
                &attempt.entry,
                Some(format!("Bearer {}", tokens.access).into_bytes()),
            ),
        )
        .await
        .unwrap_or("timeout");
        if result != "connected" {
            eprintln!("MCP OAuth: connection verification after sign-in failed ({result})");
        }
        return (result == "connected").then_some((attempt.entry, sealed));
    }
    let result = (attempt.entry.clone(), Vec::new());
    tokio::task::spawn_blocking(move || {
        principal.active(&app.store)
            && app
                .store
                .save_remote_mcp(attempt.entry, revision, Some(sealed), false)
                .is_ok()
    })
    .await
    .unwrap_or(false)
    .then_some(result)
}

fn page(success: bool) -> Response {
    let message = if success {
        "Anmeldung abgeschlossen. Dieses Fenster kann geschlossen werden."
    } else {
        "Anmeldung nicht abgeschlossen. Bitte in YoloForge erneut anmelden."
    };
    (
        if success {
            StatusCode::OK
        } else {
            StatusCode::BAD_REQUEST
        },
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; frame-ancestors 'none'",
            ),
        ],
        axum::response::Html(format!(
            "<!doctype html><meta charset=utf-8><title>YoloForge</title><p>{message}</p>"
        )),
    )
        .into_response()
}

fn encode(gateway: &Gateway, id: &str, tokens: &Tokens) -> Option<Vec<u8>> {
    gateway.seal(id, serde_json::to_vec(tokens).ok()?)
}

fn decode(gateway: &Gateway, id: &str, sealed: Vec<u8>) -> Option<Tokens> {
    serde_json::from_slice(&gateway.decrypt(id, sealed)?).ok()
}

async fn exchange(
    entry: &RemoteMcp,
    grant: &[(&str, &str)],
    prior_refresh: Option<&str>,
) -> Option<Tokens> {
    let RemoteMcpAuth::OAuth {
        client_id,
        token_url,
        ..
    } = &entry.auth
    else {
        return None;
    };
    let url = valid_url(token_url)?;
    let resource = valid_url(&entry.url)?;
    let client = upstream_with_policy(&url, resource.origin() != url.origin()).await?;
    let mut form = grant.to_vec();
    form.extend([
        ("client_id", client_id.as_str()),
        ("resource", entry.url.as_str()),
    ]);
    exchange_at(&client, url, &form, prior_refresh).await
}

async fn exchange_at(
    client: &reqwest::Client,
    url: url::Url,
    form: &[(&str, &str)],
    prior_refresh: Option<&str>,
) -> Option<Tokens> {
    let mut reply = client
        .post(url)
        .form(form)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !reply.status().is_success() {
        eprintln!(
            "MCP OAuth: token endpoint returned HTTP {}",
            reply.status().as_u16()
        );
        return None;
    }
    let mut response = Vec::new();
    while let Some(chunk) = reply.chunk().await.ok()? {
        if response.len() + chunk.len() > 32 * 1024 {
            return None;
        }
        response.extend_from_slice(&chunk);
    }
    tokens_from_reply(
        serde_json::from_slice(&response).ok()?,
        prior_refresh,
        unix_ms(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> RemoteMcp {
        RemoteMcp {
            reference: RemoteMcpRef {
                id: "docs".into(),
                name: "Docs".into(),
                revision: Revision(0),
            },
            url: "https://docs.example.invalid/mcp".into(),
            auth: RemoteMcpAuth::OAuth {
                client_id: "public-client".into(),
                authorization_url: "https://login.example.invalid/authorize".into(),
                token_url: "https://login.example.invalid/token".into(),
                redirect_uri: "https://factory.example.invalid/mcp-oauth/callback".into(),
                scope: "docs.read".into(),
            },
            has_key: false,
        }
    }

    #[test]
    fn loopback_requires_the_authenticated_operator_and_matching_mcp() {
        let mut entry = entry();
        if let RemoteMcpAuth::OAuth { redirect_uri, .. } = &mut entry.auth {
            *redirect_uri = MCP_OAUTH_LOOPBACK_REDIRECT.into();
        }
        assert!(valid_settings(&entry.auth));
        let operator = DeviceId::new();
        let attempt = Attempt {
            entry,
            operator: Some(operator),
            verifier: "verifier".into(),
            expires: u64::MAX,
            staged: true,
        };
        assert!(accepts_callback(&attempt, Some(("docs", Some(operator)))));
        assert!(!accepts_callback(&attempt, None));
        assert!(!accepts_callback(&attempt, Some(("other", Some(operator)))));
        assert!(!accepts_callback(
            &attempt,
            Some(("docs", Some(DeviceId::new())))
        ));
    }

    #[tokio::test]
    async fn verified_draft_is_only_persisted_by_explicit_commit() {
        let dir = tempfile::tempdir().unwrap();
        let gateway = Gateway::open(&dir.path().join("key"), false).unwrap();
        let store = Store::open(&dir.path().join("db")).unwrap();
        let state = "a".repeat(43);
        let sealed = gateway.seal("docs", b"fixture-token".to_vec()).unwrap();
        gateway.0.oauth_results.lock().await.insert(
            state.clone(),
            Outcome {
                id: "docs".into(),
                operator: None,
                expires: unix_ms() + ATTEMPT_MS,
                saved: Some(true),
                staged: Some((entry(), sealed)),
            },
        );
        assert!(store.remote_mcp_credentials("docs").is_err());
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let app = router("fixture-admin-token", store.clone()).layer(Extension(gateway));
        let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let post = |action: &str, body: serde_json::Value| {
            client
                .post(format!("{base}/v1/mcp/remote/docs/oauth/{action}"))
                .bearer_auth("fixture-admin-token")
                .header(VERSION_HEADER, "1")
                .json(&body)
        };
        let result: serde_json::Value = post("status", serde_json::json!({"attempt":state}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(result["status"], "verified");
        assert!(result["entry"].is_null());
        assert!(store.remote_mcp_credentials("docs").is_err());
        let commit = || {
            post(
                "commit",
                serde_json::json!({"attempt":state,"name":"Renamed"}),
            )
        };
        assert!(commit().send().await.unwrap().status().is_success());
        assert!(!commit().send().await.unwrap().status().is_success());
        let (saved, secret) = store.remote_mcp_credentials("docs").unwrap();
        assert_eq!(saved.reference.name, "Renamed");
        assert!(saved.has_key && secret.is_some());
        server.abort();
    }

    #[tokio::test]
    async fn start_generates_browser_pkce_url_and_callback_consumes_state_once() {
        let dir = tempfile::tempdir().unwrap();
        let gateway = Gateway::open(&dir.path().join("key"), false).unwrap();
        let store = Store::open(&dir.path().join("db")).unwrap();
        let meta = store
            .save_remote_mcp(entry(), Revision(0), None, false)
            .unwrap();
        assert!(valid_settings(&meta.auth));
        let app = router("fixture-admin-token", store.clone()).layer(Extension(gateway.clone()));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let result = client
            .post(format!("{base}/v1/mcp/remote/docs/oauth/start"))
            .bearer_auth("fixture-admin-token")
            .header(VERSION_HEADER, "1")
            .send()
            .await
            .unwrap();
        assert_eq!(result.headers()[header::CACHE_CONTROL], "no-store");
        let value = result.json::<serde_json::Value>().await.unwrap();
        let url = url::Url::parse(value["url"].as_str().unwrap()).unwrap();
        let pairs = url.query_pairs().into_owned().collect::<BTreeMap<_, _>>();
        let state = &pairs["state"];
        assert_eq!(value["attempt"], *state);
        assert_eq!(pairs["code_challenge_method"], "S256");
        assert_eq!(pairs["resource"], meta.url);
        {
            let pending = gateway.0.oauth_pending.lock().await;
            let secret = &pending[state].verifier;
            assert_eq!(
                pairs["code_challenge"],
                URL_SAFE_NO_PAD.encode(Sha256::digest(secret))
            );
            assert!(!value.to_string().contains(secret));
        }
        let status = || {
            client
                .post(format!("{base}/v1/mcp/remote/docs/oauth/status"))
                .bearer_auth("fixture-admin-token")
                .header(VERSION_HEADER, "1")
                .json(&serde_json::json!({"attempt":state}))
        };
        assert_eq!(
            status()
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()["status"],
            "pending"
        );
        let callback = format!("{base}{CALLBACK}?state={state}&error=access_denied");
        assert_eq!(
            client.get(&callback).send().await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
        assert!(gateway.0.oauth_pending.lock().await.is_empty());
        assert_eq!(
            status()
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()["status"],
            "failed"
        );
        assert_eq!(
            client.get(&callback).send().await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
        assert!(!store.remote_mcp_credentials("docs").unwrap().0.has_key);
        server.abort();
    }

    #[tokio::test]
    async fn token_exchange_takes_rotating_refresh_tokens_and_never_changes_profile_revision() {
        let endpoint = Router::new().route("/token", post(|body: String| async move {
            let pairs = url::form_urlencoded::parse(body.as_bytes()).into_owned().collect::<BTreeMap<_,_>>();
            assert_eq!(pairs["grant_type"], "refresh_token");
            assert_eq!(pairs["refresh_token"], "previous-refresh");
            Json(serde_json::json!({"access_token":"new-access","refresh_token":"new-refresh","token_type":"Bearer","expires_in":120}))
        }));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url =
            url::Url::parse(&format!("http://{}/token", socket.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(socket, endpoint).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let rotated = exchange_at(
            &client,
            url,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", "previous-refresh"),
            ],
            Some("previous-refresh"),
        )
        .await
        .unwrap();
        assert_eq!(rotated.refresh.as_deref(), Some("new-refresh"));
        let dir = tempfile::tempdir().unwrap();
        let gateway = Gateway::open(&dir.path().join("key"), false).unwrap();
        let store = Store::open(&dir.path().join("db")).unwrap();
        let before = gateway.seal("docs", b"previous".to_vec()).unwrap();
        let meta = store
            .save_remote_mcp(entry(), Revision(0), Some(before.clone()), false)
            .unwrap();
        let after = encode(&gateway, "docs", &rotated).unwrap();
        assert!(store
            .refresh_remote_mcp_secret("docs", meta.reference.revision, &before, &after)
            .unwrap());
        assert!(!store
            .refresh_remote_mcp_secret("docs", meta.reference.revision, &before, &after)
            .unwrap());
        let (refreshed, bytes) = store.remote_mcp_credentials("docs").unwrap();
        assert_eq!(refreshed, meta);
        assert_eq!(
            header(&gateway, &store, &refreshed, bytes).await.as_deref(),
            Some(&b"Bearer new-access"[..])
        );
        server.abort();
    }
}

fn tokens_from_reply(reply: TokenReply, prior_refresh: Option<&str>, now: u64) -> Option<Tokens> {
    let token_ok = |token: &str| {
        !token.is_empty() && token.len() <= 8192 && token.bytes().all(|b| b.is_ascii_graphic())
    };
    if !reply.token_type.eq_ignore_ascii_case("bearer")
        || !token_ok(&reply.access_token)
        || reply
            .refresh_token
            .as_deref()
            .is_some_and(|token| !token_ok(token))
    {
        return None;
    }
    Some(Tokens {
        access: reply.access_token,
        refresh: reply
            .refresh_token
            .or_else(|| prior_refresh.map(str::to_owned)),
        expires: now + reply.expires_in.unwrap_or(3600).clamp(1, 31_536_000) * 1000,
    })
}

pub(super) async fn header(
    gateway: &Gateway,
    store: &Store,
    meta: &RemoteMcp,
    sealed: Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let id = &meta.reference.id;
    let tokens = decode(gateway, id, sealed?)?;
    if tokens.expires > unix_ms().saturating_add(30_000) {
        return Some(format!("Bearer {}", tokens.access).into_bytes());
    }
    if tokens.refresh.is_none() {
        return (tokens.expires > unix_ms())
            .then(|| format!("Bearer {}", tokens.access).into_bytes());
    }
    let _guard = gateway.0.oauth_refresh.lock().await;
    let store_read = store.clone();
    let search = id.clone();
    let (current, ciphertext) =
        tokio::task::spawn_blocking(move || store_read.remote_mcp_credentials(&search))
            .await
            .ok()?
            .ok()?;
    if &current != meta {
        return None;
    }
    let ciphertext = ciphertext?;
    let tokens = decode(gateway, id, ciphertext.clone())?;
    if tokens.expires > unix_ms().saturating_add(30_000) {
        return Some(format!("Bearer {}", tokens.access).into_bytes());
    }
    let refresh = tokens.refresh.as_deref()?;
    let updated = exchange(
        meta,
        &[("grant_type", "refresh_token"), ("refresh_token", refresh)],
        Some(refresh),
    )
    .await;
    let Some(updated) = updated else {
        return (tokens.expires > unix_ms())
            .then(|| format!("Bearer {}", tokens.access).into_bytes());
    };
    let replacement = encode(gateway, id, &updated)?;
    let store = store.clone();
    let id = id.clone();
    let revision = meta.reference.revision;
    if !tokio::task::spawn_blocking(move || {
        store.refresh_remote_mcp_secret(&id, revision, &ciphertext, &replacement)
    })
    .await
    .ok()?
    .ok()?
    {
        return None;
    }
    Some(format!("Bearer {}", updated.access).into_bytes())
}
