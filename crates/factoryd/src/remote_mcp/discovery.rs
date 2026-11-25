use super::*;
use factory_protocol::mcp::{
    ProbeRemoteMcpAuth, RemoteMcpAuthDiscovery as Discovery, MCP_OAUTH_LOOPBACK_REDIRECT,
};
use serde::Deserialize;
use url::Url;

const CLIENT: &str = "/mcp-oauth/client.json";
const CALLBACK: &str = "/mcp-oauth/callback";

pub(crate) async fn discover(
    Extension(principal): Extension<Principal>,
    gateway: Option<Extension<Gateway>>,
    body: Result<Json<ProbeRemoteMcpAuth>, JsonRejection>,
) -> Response {
    if !only_admin(&principal) {
        return denied();
    }
    let Some(Extension(gateway)) = gateway else {
        return missing();
    };
    let Some(resource) = body.ok().and_then(|Json(body)| valid_url(&body.url)) else {
        return bad();
    };
    let result = tokio::time::timeout(Duration::from_secs(12), probe(&gateway, &resource))
        .await
        .unwrap_or(Discovery::Unavailable);
    Json(result).into_response()
}

pub(crate) async fn client_metadata(gateway: Option<Extension<Gateway>>) -> Response {
    let Some(Extension(gateway)) = gateway else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(origin) = gateway.0.oauth_origin.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let client = origin.join(CLIENT).expect("validated origin");
    let redirect = origin.join(CALLBACK).expect("validated origin");
    Json(client_document(Some(client.as_str()), &redirect)).into_response()
}

async fn probe(gateway: &Gateway, resource: &Url) -> Discovery {
    let Some(client) = upstream(resource).await else {
        return Discovery::Unavailable;
    };
    let post = |message| {
        Some(
            client
                .post(resource.clone())
                .header(header::ACCEPT, "application/json, text/event-stream")
                .json(&message),
        )
    };
    let Ok(response) = post(initialize_request()).unwrap().send().await else {
        return Discovery::Unavailable;
    };
    let protected = match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => response.headers().clone(),
        status if status.is_success() => {
            match finish_probe(response, post, Some(client.delete(resource.clone()))).await {
                ProbeResult::Connected => return Discovery::Open,
                ProbeResult::NeedsAuth(headers) => headers,
                ProbeResult::Unavailable => return Discovery::Unavailable,
            }
        }
        _ => return Discovery::Unavailable,
    };
    let challenge = challenge(&protected);
    match oauth_metadata(resource, &challenge, gateway.0.oauth_origin.as_ref()).await {
        Some(auth) => Discovery::OAuth { auth },
        None => Discovery::Unknown,
    }
}

#[derive(Default, Debug)]
struct Challenge {
    metadata: Option<String>,
    scope: Option<String>,
}

fn challenge(headers: &HeaderMap) -> Challenge {
    let mut result = Challenge::default();
    for value in headers.get_all(header::WWW_AUTHENTICATE) {
        let Ok(value) = value.to_str() else { continue };
        let mut quoted = false;
        let mut escaped = false;
        let fields = value.split(|c| {
            let separator = c == ',' && !quoted;
            if c == '"' && !escaped {
                quoted = !quoted;
            }
            escaped = c == '\\' && !escaped && quoted;
            separator
        });
        let mut bearer = false;
        for field in fields {
            let mut field = field.trim();
            let first = field.find(|c: char| c.is_ascii_whitespace() || c == '=');
            let first_token = &field[..first.unwrap_or(field.len())];
            let remainder = field[first_token.len()..].trim_start();
            if !remainder.starts_with('=') {
                bearer = first_token.eq_ignore_ascii_case("bearer");
                field = remainder;
            }
            if !bearer {
                continue;
            }
            let Some((key, value)) = field.split_once('=') else {
                continue;
            };
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(value);
            if key.trim().eq_ignore_ascii_case("resource_metadata") {
                result.metadata.get_or_insert_with(|| value.to_owned());
            }
            if key.trim().eq_ignore_ascii_case("scope") {
                result.scope.get_or_insert_with(|| value.to_owned());
            }
        }
    }
    result
}

#[derive(Deserialize)]
struct ResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

#[derive(Deserialize)]
struct IssuerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    client_id_metadata_document_supported: bool,
    registration_endpoint: Option<String>,
}

#[derive(Deserialize)]
struct RegisteredClient {
    client_id: String,
    token_endpoint_auth_method: Option<String>,
    client_secret: Option<String>,
    redirect_uris: Vec<String>,
}

fn client_document(client_id: Option<&str>, redirect: &Url) -> serde_json::Value {
    let mut document = serde_json::json!({
        "client_name": "YoloForge",
        "redirect_uris": [redirect.as_str()],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    });
    if let Some(id) = client_id {
        document["client_id"] = serde_json::json!(id);
    }
    document
}

async fn register_client(endpoint: &str, redirect: &Url) -> Option<String> {
    let endpoint = valid_url(endpoint)?;
    let client = upstream_with_policy(&endpoint, true).await?;
    let response = client
        .post(endpoint)
        .header(header::ACCEPT, "application/json")
        .json(&client_document(None, redirect))
        .timeout(Duration::from_secs(2))
        .send()
        .await;
    let Ok(response) = response else {
        eprintln!("MCP OAuth: client registration endpoint could not be reached");
        return None;
    };
    if !response.status().is_success() {
        eprintln!(
            "MCP OAuth: client registration returned HTTP {}",
            response.status().as_u16()
        );
        return None;
    }
    let client_id = read_metadata::<RegisteredClient>(response)
        .await
        .and_then(|registered| registered.public_id(redirect));
    if client_id.is_none() {
        eprintln!("MCP OAuth: registration did not return a compatible public client");
    }
    client_id
}

impl RegisteredClient {
    fn public_id(self, redirect: &Url) -> Option<String> {
        (!self.client_id.is_empty()
            && self.client_id.len() <= 512
            && self.client_id.bytes().all(|b| b.is_ascii_graphic())
            && self.token_endpoint_auth_method.as_deref() == Some("none")
            && self.client_secret.is_none()
            && self
                .redirect_uris
                .iter()
                .any(|uri| uri == redirect.as_str()))
        .then_some(self.client_id)
    }
}

fn well_known(origin: &Url, name: &str, suffix: &str) -> Url {
    // Build by replacing the path, never by resolving a server-controlled relative reference.
    let mut url = origin.clone();
    url.set_path(&format!("/.well-known/{name}{suffix}"));
    url
}

async fn oauth_metadata(
    resource: &Url,
    challenge: &Challenge,
    origin: Option<&Url>,
) -> Option<RemoteMcpAuth> {
    let mut candidates = vec![];
    if let Some(metadata) = &challenge.metadata {
        candidates.push(valid_url(metadata)?);
    } else {
        if resource.path() != "/" {
            candidates.push(well_known(
                resource,
                "oauth-protected-resource",
                resource.path(),
            ));
        }
        candidates.push(well_known(resource, "oauth-protected-resource", ""));
    }
    let mut protected = None;
    for url in candidates {
        if let Some(metadata) = metadata::<ResourceMetadata>(&url, resource).await {
            if valid_url(&metadata.resource).as_ref() == Some(resource) {
                protected = Some(metadata);
                break;
            }
        }
    }
    let protected = protected?;
    let scope = challenge
        .scope
        .clone()
        .unwrap_or_else(|| protected.scopes_supported.join(" "));
    if scope.len() > 512
        || !scope
            .bytes()
            .all(|byte| byte == b' ' || byte.is_ascii_graphic())
    {
        return None;
    }
    for issuer in protected
        .authorization_servers
        .iter()
        .take(3)
        .filter_map(|s| valid_url(s))
    {
        let path = if issuer.path() == "/" {
            ""
        } else {
            issuer.path()
        };
        let mut appended = issuer.clone();
        appended.set_path(&format!(
            "{}/.well-known/openid-configuration",
            path.trim_end_matches('/')
        ));
        for endpoint in [
            well_known(&issuer, "oauth-authorization-server", path),
            well_known(&issuer, "openid-configuration", path),
            appended,
        ] {
            let Some(config) = metadata::<IssuerMetadata>(&endpoint, resource).await else {
                continue;
            };
            if valid_url(&config.issuer).as_ref() != Some(&issuer)
                || !config
                    .code_challenge_methods_supported
                    .iter()
                    .any(|method| method == "S256")
                || valid_url(&config.authorization_endpoint).is_none()
                || valid_url(&config.token_endpoint).is_none()
            {
                continue;
            }
            let redirect = origin
                .and_then(|url| url.join(CALLBACK).ok())
                .unwrap_or_else(|| {
                    Url::parse(MCP_OAUTH_LOOPBACK_REDIRECT).expect("fixed redirect")
                });
            let client_id = if origin.is_some() && config.client_id_metadata_document_supported {
                origin
                    .and_then(|url| url.join(CLIENT).ok())
                    .map(|url| url.to_string())
            } else if let Some(endpoint) = config.registration_endpoint.as_deref() {
                tokio::time::timeout(Duration::from_secs(3), register_client(endpoint, &redirect))
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            };
            return Some(RemoteMcpAuth::OAuth {
                client_id: client_id.unwrap_or_default(),
                redirect_uri: redirect.to_string(),
                authorization_url: config.authorization_endpoint,
                token_url: config.token_endpoint,
                scope,
            });
        }
    }
    None
}

async fn metadata<T: serde::de::DeserializeOwned>(url: &Url, resource: &Url) -> Option<T> {
    let client = upstream_with_policy(url, url.origin() != resource.origin()).await?;
    let response = client
        .get(url.clone())
        .header(header::ACCEPT, "application/json")
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?;
    read_metadata(response).await
}

async fn read_metadata<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> Option<T> {
    if !response.status().is_success() {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len() + chunk.len() > 32 * 1024 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_documentation()
                && !(ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
                && ip.octets()[0] < 224
        }
        IpAddr::V6(ip) => ip.to_ipv4().map_or_else(
            || (ip.segments()[0] & 0xe000) == 0x2000,
            |ip| public_address(ip.into()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    #[ignore = "live OpenArt smoke test; registers an OAuth client"]
    async fn openart_discovery_and_prepare_return_a_browser_url() {
        let dir = tempfile::tempdir().unwrap();
        let gateway = Gateway::open(&dir.path().join("key"), false).unwrap();
        let store = Store::open(&dir.path().join("db")).unwrap();
        let app = router("fixture-admin-token", store.clone()).layer(Extension(gateway));
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", socket.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let resource = "https://mcp.openart.ai/mcp";
        let post = |path: &str, body: serde_json::Value| {
            client
                .post(format!("{base}{path}"))
                .bearer_auth("fixture-admin-token")
                .header(VERSION_HEADER, "1")
                .json(&body)
        };
        let result = post(
            "/v1/mcp/remote/discover",
            serde_json::json!({"url":resource}),
        )
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Discovery>()
        .await
        .unwrap();
        let Discovery::OAuth { auth } = result else {
            panic!("OpenArt OAuth was not discovered")
        };
        assert!(
            oauth::valid_settings(&auth),
            "OpenArt registration did not produce usable OAuth settings"
        );
        let reply: serde_json::Value = post(
            "/v1/mcp/remote/openart/oauth/prepare",
            serde_json::json!({
                "expected_revision": 0, "name": "OpenArt", "url": resource, "auth": auth,
            }),
        )
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
        assert!(reply["url"].as_str().is_some_and(
            |url| url.starts_with("https://openart.ai/suite/api/auth/oauth/authorize?")
        ));
        assert!(store.remote_mcp_credentials("openart").is_err());
        server.abort();
    }

    #[test]
    fn dynamic_registration_requires_a_public_client_and_the_factory_callback() {
        let redirect = Url::parse("https://factory.example/mcp-oauth/callback").unwrap();
        let mut response = client_document(None, &redirect);
        response["client_id"] = serde_json::json!("registered-id");
        let parse = |response| {
            serde_json::from_value::<RegisteredClient>(response)
                .unwrap()
                .public_id(&redirect)
        };
        assert_eq!(parse(response.clone()).as_deref(), Some("registered-id"));
        response["client_secret"] = serde_json::json!("unexpected-secret");
        assert!(parse(response.clone()).is_none());
        response["client_secret"] = serde_json::Value::Null;
        response["redirect_uris"] = serde_json::json!(["https://other.example/callback"]);
        assert!(parse(response).is_none());
    }

    #[test]
    fn bearer_parameters_are_read_without_mistaking_a_bare_challenge_for_oauth() {
        let mut headers = HeaderMap::new();
        headers.append(header::WWW_AUTHENTICATE, "Bearer".parse().unwrap());
        assert!(challenge(&headers).metadata.is_none());
        headers.append(
            header::WWW_AUTHENTICATE,
            r#"Basic realm="other, realm", Bearer resource_metadata = "https://mcp.example/meta", scope="read write""#
                .parse()
                .unwrap(),
        );
        let parsed = challenge(&headers);
        assert_eq!(parsed.metadata.as_deref(), Some("https://mcp.example/meta"));
        assert_eq!(parsed.scope.as_deref(), Some("read write"));
    }

    #[tokio::test]
    async fn client_metadata_uses_the_configured_public_origin() {
        let dir = tempfile::tempdir().unwrap();
        let gateway = Gateway::open(&dir.path().join("mcp.key"), false)
            .unwrap()
            .with_oauth_origin("https://factory.example")
            .unwrap();
        let response = client_metadata(Some(Extension(gateway))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let data: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(
            data["client_id"],
            "https://factory.example/mcp-oauth/client.json"
        );
        assert_eq!(
            data["redirect_uris"][0],
            "https://factory.example/mcp-oauth/callback"
        );
        assert_eq!(data["token_endpoint_auth_method"], "none");
    }
}
