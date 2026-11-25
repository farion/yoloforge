use super::{error, login_context, private_json, Gateway};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use factory_core::RequestId;
use factory_protocol::models::{KeyModelProvider as Provider, ModelAccess};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{net::IpAddr, time::Duration};

const GOOGLE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const ROUTER: &str = "https://openrouter.ai/api/v1";
const ANTHROPIC: &str = "https://api.anthropic.com/v1";

pub(super) struct KeyCredential {
    key: String,
    endpoint: Option<String>,
    custom: bool,
}

pub(super) async fn credential(
    gateway: &Gateway,
    provider: Provider,
    id: &str,
) -> Option<KeyCredential> {
    let entry = gateway.entries().await?.into_iter().find(|entry| {
        entry.id == id
            && entry.integration_id == provider.integration()
            && entry.value["type"] == "key"
    })?;
    let custom = entry.value["metadata"]["customEndpoint"] == true;
    let endpoint = if provider.endpoint_env().is_some() {
        let saved = entry.value["metadata"]["endpoint"].as_str()?;
        let normalized = validated_endpoint(saved)?;
        if !custom && configured_endpoint(provider)? != normalized {
            return None;
        }
        Some(normalized)
    } else {
        None
    };
    let key = entry.value["key"]
        .as_str()
        .filter(|key| valid_key(provider, key))?
        .to_owned();
    Some(KeyCredential {
        key,
        endpoint,
        custom,
    })
}

fn valid_key(provider: Provider, key: &str) -> bool {
    (key.is_empty() && provider.key_optional())
        || ((1..=4096).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_graphic()))
}

fn configured_endpoint(provider: Provider) -> Option<String> {
    normalize_endpoint(&std::env::var(provider.endpoint_env()?).ok()?)
}

fn normalize_endpoint(value: &str) -> Option<String> {
    let base = validated_endpoint(value)?;
    Some(base.strip_suffix("/v1").unwrap_or(&base).to_owned())
}

fn validated_endpoint(value: &str) -> Option<String> {
    let url = url::Url::parse(value.trim()).ok()?;
    (value.len() <= 1024
        && matches!(url.scheme(), "http" | "https")
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| url.as_str().trim_end_matches('/').to_owned())
}

fn safe_endpoint_ip(ip: IpAddr, tls: bool) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_link_local()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_unspecified()
                && ip.octets()[0] != 0
                && ip.octets() != [100, 100, 100, 200]
                && (tls || ip.is_private() || ip.is_loopback())
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return safe_endpoint_ip(mapped.into(), tls);
            }
            !ip.is_unicast_link_local()
                && !ip.is_multicast()
                && !ip.is_unspecified()
                && (tls || ip.is_unique_local() || ip.is_loopback())
        }
    }
}

async fn upstream_client(gateway: &Gateway, credential: &KeyCredential) -> Option<reqwest::Client> {
    if !credential.custom {
        return Some(gateway.0.client.clone());
    }
    let url = url::Url::parse(credential.endpoint.as_deref()?).ok()?;
    let host = url.host()?;
    let tls = url.scheme() == "https";
    let mut client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(310));
    match host {
        url::Host::Domain(host) => {
            let addresses: Vec<_> = tokio::time::timeout(
                Duration::from_secs(4),
                tokio::net::lookup_host((host, url.port_or_known_default()?)),
            )
            .await
            .ok()?
            .ok()?
            .collect();
            if addresses.is_empty()
                || addresses
                    .iter()
                    .any(|address| !safe_endpoint_ip(address.ip(), tls))
            {
                return None;
            }
            // Pin the validated DNS result for this request, including model discovery.
            client = client.resolve_to_addrs(host, &addresses);
        }
        url::Host::Ipv4(ip) if safe_endpoint_ip(ip.into(), tls) => {}
        url::Host::Ipv6(ip) if safe_endpoint_ip(ip.into(), tls) => {}
        _ => return None,
    }
    client.build().ok()
}

fn authenticate(
    request: reqwest::RequestBuilder,
    provider: Provider,
    secret: &str,
) -> reqwest::RequestBuilder {
    match provider {
        Provider::Gemini => request.header("x-goog-api-key", secret),
        Provider::OpenRouter | Provider::LiteLlm | Provider::Ollama if !secret.is_empty() => {
            request.bearer_auth(secret)
        }
        Provider::OpenRouter | Provider::LiteLlm | Provider::Ollama => request,
        Provider::Anthropic => request
            .header("x-api-key", secret)
            .header("anthropic-version", "2023-06-01"),
    }
}

fn supports_tools(item: &Value) -> bool {
    item["supported_parameters"]
        .as_array()
        .is_some_and(|params| params.iter().any(|p| p == "tools"))
}

fn model_ids(provider: Provider, payload: &Value) -> Vec<String> {
    let entries = match provider {
        Provider::Gemini | Provider::Ollama => &payload["models"],
        _ => &payload["data"],
    };
    let mut ids: Vec<_> = entries
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| match provider {
            Provider::Gemini => {
                let id = item["name"].as_str()?.strip_prefix("models/gemini-")?;
                let generates = item["supportedGenerationMethods"]
                    .as_array()?
                    .iter()
                    .any(|method| method == "generateContent");
                let special = [
                    "image", "tts", "audio", "live", "embed", "robotics", "computer",
                ]
                .iter()
                .any(|part| id.split('-').any(|s| s == *part));
                (generates && id.starts_with(|c: char| c.is_ascii_digit()) && !special)
                    .then(|| format!("gemini-{id}"))
            }
            Provider::OpenRouter => {
                let text_output = item["architecture"]["output_modalities"]
                    .as_array()?
                    .iter()
                    .any(|p| p == "text");
                (supports_tools(item) && text_output).then_some(item["id"].as_str()?.to_owned())
            }
            Provider::Anthropic => item["id"]
                .as_str()
                .filter(|id| id.starts_with("claude-"))
                .map(str::to_owned),
            Provider::LiteLlm => item["id"].as_str().map(str::to_owned),
            Provider::Ollama => item["name"].as_str().map(str::to_owned),
        })
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 200
                && (provider == Provider::LiteLlm && !id.chars().any(char::is_control)
                    || id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.:/".contains(&c)))
                && (provider != Provider::Gemini || !id.contains(['/', ':']))
        })
        .collect();
    ids.sort_unstable_by(|a, b| {
        let priority = |id: &str| match provider {
            Provider::Gemini if id == "gemini-4-hydrogen" => 0,
            Provider::Gemini if id.contains("flash") && !id.contains("preview") => 1,
            Provider::Anthropic if id.contains("sonnet") => 0,
            Provider::OpenRouter if id.ends_with(":free") => 0,
            _ => 2,
        };
        priority(a).cmp(&priority(b)).then_with(|| a.cmp(b))
    });
    ids.dedup();
    ids.truncate(256);
    ids
}

async fn discover(
    gateway: &Gateway,
    provider: Provider,
    credential: &KeyCredential,
) -> Result<Vec<String>, Response> {
    let unavailable = || {
        error(
            StatusCode::BAD_GATEWAY,
            "Der Modell-Anbieter ist gerade nicht erreichbar.",
        )
    };
    let client = upstream_client(gateway, credential).await.ok_or_else(|| error(
        StatusCode::BAD_REQUEST, "Die Server-URL ist nicht erreichbar oder nicht zulässig. Öffentliche Server benötigen HTTPS."
    ))?;
    let request = |url: String| {
        authenticate(client.get(url), provider, &credential.key).timeout(Duration::from_secs(12))
    };
    let list = match provider {
        Provider::Gemini => format!("{GOOGLE}?pageSize=1000"),
        Provider::OpenRouter => {
            // The public model list alone does not validate an API key.
            let response = request(format!("{ROUTER}/key"))
                .send()
                .await
                .map_err(|_| unavailable())?;
            check_discovery_status(response.status())?;
            format!("{ROUTER}/models")
        }
        Provider::Anthropic => format!("{ANTHROPIC}/models?limit=100"),
        Provider::LiteLlm | Provider::Ollama => {
            let base = credential.endpoint.as_deref().ok_or_else(unavailable)?;
            let path = if provider == Provider::Ollama {
                "/api/tags"
            } else {
                "/v1/models"
            };
            format!("{base}{path}")
        }
    };
    let response = request(list).send().await.map_err(|_| unavailable())?;
    check_discovery_status(response.status())?;
    let payload = read_json(response).await.ok_or_else(unavailable)?;
    let mut models = model_ids(provider, &payload);
    if provider == Provider::Ollama {
        use futures_util::{stream, StreamExt};
        let base = credential.endpoint.as_deref().ok_or_else(unavailable)?;
        let checks = stream::iter(models.into_iter().map(|model| {
            let request = authenticate(
                client.post(format!("{base}/api/show")),
                provider,
                &credential.key,
            )
            .timeout(Duration::from_secs(5))
            .json(&json!({"model": model}));
            async move {
                let info = read_json(request.send().await.ok()?.error_for_status().ok()?).await?;
                info["capabilities"]
                    .as_array()?
                    .iter()
                    .any(|capability| capability == "tools")
                    .then_some(model)
            }
        }))
        .buffer_unordered(8)
        .filter_map(std::future::ready)
        .collect::<Vec<_>>();
        let mut supported = tokio::time::timeout(Duration::from_secs(18), checks)
            .await
            .map_err(|_| unavailable())?;
        supported.sort_unstable();
        models = supported;
    }
    if models.is_empty() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Für diesen Zugang wurden keine passenden Modelle gefunden.",
        ));
    }
    Ok(models)
}

pub(super) async fn read_json(mut response: reqwest::Response) -> Option<Value> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).ok()
}

fn check_discovery_status(status: StatusCode) -> Result<(), Response> {
    match status {
        status if status.is_success() => Ok(()),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::BAD_REQUEST => Err(error(
            StatusCode::BAD_REQUEST,
            "Der Anbieter hat den API-Schlüssel abgelehnt. Prüfe den Schlüssel und seine Berechtigungen.",
        )),
        _ => Err(error(StatusCode::BAD_GATEWAY, "Die Modelle des Anbieters konnten nicht geladen werden.")),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Connect {
    name: String,
    key: String,
    url: Option<String>,
}

pub(crate) async fn connect(
    State(app): State<crate::App>,
    Extension(principal): Extension<crate::Principal>,
    gateway: Option<Extension<Gateway>>,
    Path(alias): Path<String>,
    Json(body): Json<Connect>,
) -> Response {
    let (gateway, store) = match login_context(app, principal, gateway) {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let Some(provider) = Provider::from_access_id(&alias) else {
        return error(StatusCode::BAD_REQUEST, "Unbekannter Modell-Anbieter.");
    };
    let name = body.name.trim();
    let key = body.key.trim();
    if !crate::valid_text(name, 120) || !valid_key(provider, key) {
        return error(
            StatusCode::BAD_REQUEST,
            "Name oder API-Schlüssel ist ungültig.",
        );
    }
    let (endpoint, custom) = match (provider.endpoint_env(), body.url.as_deref()) {
        (Some(_), Some(url)) => {
            let Some(endpoint) = normalize_endpoint(url) else {
                return error(StatusCode::BAD_REQUEST, "Gib eine gültige Server-URL ein.");
            };
            (Some(endpoint), true)
        }
        (Some(_), None) => {
            let Some(endpoint) = configured_endpoint(provider) else {
                return error(
                    StatusCode::BAD_REQUEST,
                    "Gib die Server-URL des Anbieters ein.",
                );
            };
            (Some(endpoint), false)
        }
        (None, Some(_)) => {
            return error(
                StatusCode::BAD_REQUEST,
                "Dieser Anbieter benötigt keine Server-URL.",
            )
        }
        (None, None) => (None, false),
    };
    // Finish saving (or deleting the unused credential) even if the desktop disconnects.
    let name = name.to_owned();
    let credential = KeyCredential {
        key: key.to_owned(),
        endpoint,
        custom,
    };
    tokio::spawn(async move {
        let catalog = store.clone();
        let Ok(Ok(settings)) = tokio::task::spawn_blocking(move || catalog.factory_models()).await
        else {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Die Modellzugänge konnten nicht geladen werden.",
            );
        };
        if settings.accesses.len() >= 64
            || settings.accesses.iter().any(|access| {
                access.id == alias || access.name.trim().to_lowercase() == name.to_lowercase()
            })
        {
            return error(
                StatusCode::CONFLICT,
                "Der Name ist bereits vergeben oder das Zugangslimit ist erreicht.",
            );
        }
        let models = match discover(&gateway, provider, &credential).await {
            Ok(models) => models,
            Err(response) => return response,
        };
        let id = RequestId::new().to_string();
        let central = gateway.0.central.url.trim_end_matches('/');
        let saved = gateway
            .0
            .client
            .post(format!("{central}/api/credential"))
            .basic_auth("opencode", Some(&gateway.0.central.password))
            .timeout(Duration::from_secs(5))
            .json(&json!({
                "id": id, "integrationID": provider.integration(), "label": name,
                "value": {"type": "key", "key": credential.key, "metadata": {
                    "endpoint": credential.endpoint, "customEndpoint": credential.custom
                }}, "activate": false
            }))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        if saved {
            let access = ModelAccess {
                id: alias,
                name,
                models,
                transports: Default::default(),
            };
            let binding = id.clone();
            if let Ok(Ok(default)) = tokio::task::spawn_blocking(move || {
                store.add_connected_model_access(access, &binding)
            })
            .await
            {
                return private_json(json!({"connected":true,"automatic_default":default}));
            }
        }
        let _ = gateway
            .0
            .client
            .delete(format!("{central}/api/credential/{id}"))
            .basic_auth("opencode", Some(&gateway.0.central.password))
            .timeout(Duration::from_secs(5))
            .send()
            .await;
        error(
            StatusCode::BAD_GATEWAY,
            "Der Modellzugang konnte nicht gespeichert werden.",
        )
    })
    .await
    .unwrap_or_else(|_| {
        error(
            StatusCode::BAD_GATEWAY,
            "Der Modellzugang konnte nicht gespeichert werden.",
        )
    })
}

type GatewayState = (Gateway, crate::store::Store);

pub(super) async fn chat(
    State(state): State<GatewayState>,
    Path(alias): Path<String>,
    headers: HeaderMap,
    Json(data): Json<Value>,
) -> Response {
    if factory_protocol::models::is_copilot_access(&alias) {
        return super::copilot::relay(
            state,
            alias,
            headers,
            data,
            factory_protocol::models::ModelTransport::Chat,
        )
        .await;
    }
    let Some(provider @ (Provider::OpenRouter | Provider::LiteLlm | Provider::Ollama)) =
        Provider::from_access_id(&alias)
    else {
        return error(StatusCode::NOT_FOUND, "Factory model access unavailable");
    };
    relay(state, alias, headers, data, provider, None).await
}

pub(super) async fn messages(
    State(state): State<GatewayState>,
    Path(alias): Path<String>,
    headers: HeaderMap,
    Json(data): Json<Value>,
) -> Response {
    if factory_protocol::models::is_copilot_access(&alias) {
        return super::copilot::relay(
            state,
            alias,
            headers,
            data,
            factory_protocol::models::ModelTransport::Messages,
        )
        .await;
    }
    relay(state, alias, headers, data, Provider::Anthropic, None).await
}

pub(super) async fn gemini(
    State(state): State<GatewayState>,
    Path((alias, operation)): Path<(String, String)>,
    headers: HeaderMap,
    Json(data): Json<Value>,
) -> Response {
    let Some((model, action @ ("generateContent" | "streamGenerateContent"))) =
        operation.split_once(':')
    else {
        return error(StatusCode::NOT_FOUND, "Unsupported Gemini operation");
    };
    if model.is_empty()
        || !model
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return error(StatusCode::BAD_REQUEST, "Invalid Gemini model");
    }
    relay(
        state,
        alias,
        headers,
        data,
        Provider::Gemini,
        Some((model, action)),
    )
    .await
}

async fn relay(
    (gateway, store): GatewayState,
    alias: String,
    mut headers: HeaderMap,
    data: Value,
    provider: Provider,
    gemini: Option<(&str, &str)>,
) -> Response {
    // Native SDKs send the Factory key in their provider-specific header.
    if !headers.contains_key(header::AUTHORIZATION) {
        let native = match provider {
            Provider::Gemini => headers.get("x-goog-api-key"),
            Provider::Anthropic => headers.get("x-api-key"),
            Provider::OpenRouter | Provider::LiteLlm | Provider::Ollama => None,
        };
        if let Some(bearer) = native
            .and_then(|key| key.to_str().ok())
            .and_then(|key| HeaderValue::from_str(&format!("Bearer {key}")).ok())
        {
            headers.insert(header::AUTHORIZATION, bearer);
        }
    }
    if !gateway.allows_runner(&headers, &store).await {
        return error(StatusCode::UNAUTHORIZED, "Factory model key required");
    }
    if Provider::from_access_id(&alias) != Some(provider) {
        return error(StatusCode::NOT_FOUND, "Factory model access unavailable");
    }
    if matches!(provider, Provider::OpenRouter | Provider::LiteLlm) && data.get("models").is_some()
    {
        return error(
            StatusCode::BAD_REQUEST,
            "Alternate models are not permitted",
        );
    }
    if provider == Provider::LiteLlm && !is_litellm_chat_request(&data) {
        return error(
            StatusCode::BAD_REQUEST,
            "Unsupported LiteLLM chat parameters",
        );
    }
    let model = gemini
        .map(|(model, _)| model)
        .or_else(|| data["model"].as_str());
    let Some(model) = model.map(str::to_owned) else {
        return error(StatusCode::BAD_REQUEST, "Model required");
    };
    let access = alias.clone();
    match tokio::task::spawn_blocking(move || {
        store.factory_models().and_then(|settings| {
            if settings
                .accesses
                .iter()
                .any(|a| a.id == access && a.models.contains(&model))
            {
                store.model_binding(&access)
            } else {
                Ok(None)
            }
        })
    })
    .await
    {
        Ok(Ok(Some(id))) => {
            let Some(credential) = credential(&gateway, provider, &id).await else {
                return error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Factory model access not ready",
                );
            };
            let upstream = match provider {
                Provider::Gemini => {
                    let (model, action) = gemini.expect("Gemini route supplies model");
                    let query = if action == "streamGenerateContent" {
                        "?alt=sse"
                    } else {
                        ""
                    };
                    format!("{GOOGLE}/{model}:{action}{query}")
                }
                Provider::OpenRouter => format!("{ROUTER}/chat/completions"),
                Provider::Anthropic => format!("{ANTHROPIC}/messages"),
                Provider::LiteLlm | Provider::Ollama => {
                    let Some(base) = credential.endpoint.as_deref() else {
                        return error(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "Factory model access not ready",
                        );
                    };
                    format!("{base}/v1/chat/completions")
                }
            };
            let Some(client) = upstream_client(&gateway, &credential).await else {
                return error(
                    StatusCode::BAD_GATEWAY,
                    "Model provider address unavailable",
                );
            };
            let response = authenticate(client.post(upstream), provider, &credential.key)
                .json(&data)
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => {
                    let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
                    let mut result = Body::from_stream(response.bytes_stream()).into_response();
                    if let Some(value) = content_type {
                        result.headers_mut().insert(header::CONTENT_TYPE, value);
                    }
                    result
                }
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
        Ok(Ok(None)) => error(StatusCode::FORBIDDEN, "Model not permitted for this access"),
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Factory model catalog unavailable",
        ),
    }
}

fn is_litellm_chat_request(data: &Value) -> bool {
    // Do not allow runner-supplied LiteLLM extensions such as api_base,
    // api_key, model fallbacks or arbitrary upstream headers.
    data.as_object().is_some_and(|body| {
        body.keys().all(|key| {
            matches!(
                key.as_str(),
                "model"
                    | "messages"
                    | "tools"
                    | "tool_choice"
                    | "parallel_tool_calls"
                    | "stream"
                    | "stream_options"
                    | "max_tokens"
                    | "max_completion_tokens"
                    | "temperature"
                    | "top_p"
                    | "top_logprobs"
                    | "logprobs"
                    | "stop"
                    | "frequency_penalty"
                    | "presence_penalty"
                    | "seed"
                    | "response_format"
                    | "reasoning_effort"
                    | "thinking"
                    | "verbosity"
                    | "user"
                    | "n"
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn custom_local_urls_are_used_for_discovery_without_environment_overrides() {
        use axum::{
            routing::{get, post},
            Router,
        };
        let router = Router::new()
            .route("/proxy/v1/models", get(|headers: HeaderMap| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer test-litellm-key");
                Json(json!({"data":[{"id":"test-deployment"}]}))
            }))
            .route("/api/tags", get(|headers: HeaderMap| async move {
                assert!(!headers.contains_key(header::AUTHORIZATION));
                Json(json!({"models":[{"name":"test-coder:latest"},{"name":"plain:latest"}]}))
            }))
            .route("/api/show", post(|Json(body): Json<Value>| async move {
                Json(json!({"capabilities":if body["model"] == "test-coder:latest" { vec!["completion", "tools"] } else { vec!["completion"] }}))
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let gateway = Gateway::managed(
            base.clone(),
            "central-test-password-thirty-two".into(),
            "factory-model-key-at-least-thirty-two".into(),
        )
        .unwrap();
        let lite = KeyCredential {
            key: "test-litellm-key".into(),
            endpoint: normalize_endpoint(&format!("{base}/proxy/v1/")),
            custom: true,
        };
        let ollama = KeyCredential {
            key: String::new(),
            endpoint: normalize_endpoint(&base),
            custom: true,
        };
        assert_eq!(
            discover(&gateway, Provider::LiteLlm, &lite).await.unwrap(),
            ["test-deployment"]
        );
        assert_eq!(
            discover(&gateway, Provider::Ollama, &ollama).await.unwrap(),
            ["test-coder:latest"]
        );
        assert!(!safe_endpoint_ip("169.254.169.254".parse().unwrap(), true));
        assert!(!safe_endpoint_ip(
            "::ffff:169.254.169.254".parse().unwrap(),
            true
        ));
        assert!(!safe_endpoint_ip("8.8.8.8".parse().unwrap(), false));
        assert!(normalize_endpoint("https://user:pass@localhost/").is_none());
        task.abort();
    }

    #[test]
    fn discovers_chat_models_for_each_provider() {
        let gemini = json!({"models":[
            {"name":"models/gemini-4-hydrogen","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-embedding-2","supportedGenerationMethods":["embedContent"]},
            {"name":"models/gemini-4-flash-image","supportedGenerationMethods":["generateContent"]}
        ]});
        assert_eq!(model_ids(Provider::Gemini, &gemini), ["gemini-4-hydrogen"]);
        let openrouter = json!({"data":[
            {"id":"anthropic/claude-sonnet","supported_parameters":["tools"],"architecture":{"output_modalities":["text"]}},
            {"id":"google/image","supported_parameters":["tools"],"architecture":{"output_modalities":["image"]}},
            {"id":"other/no-tools","supported_parameters":["temperature"],"architecture":{"output_modalities":["text"]}}
        ]});
        assert_eq!(
            model_ids(Provider::OpenRouter, &openrouter),
            ["anthropic/claude-sonnet"]
        );
        assert_eq!(
            model_ids(
                Provider::Anthropic,
                &json!({"data":[{"id":"claude-sonnet-4"}]})
            ),
            ["claude-sonnet-4"]
        );
        assert_eq!(
            model_ids(
                Provider::LiteLlm,
                &json!({"data":[{"id":"coding / production"}]})
            ),
            ["coding / production"]
        );
        assert_eq!(
            model_ids(Provider::Ollama, &json!({"models":[{"name":"qwen3:8b"}]})),
            ["qwen3:8b"]
        );
    }

    #[test]
    fn reserves_provider_aliases_and_accepts_keyless_ollama_only() {
        for provider in Provider::ALL {
            assert_eq!(
                Provider::from_access_id(&provider.new_access_id()),
                Some(provider)
            );
            assert!(valid_key(provider, "test-key"));
            assert!(!valid_key(provider, "key\nvalue"));
            assert_eq!(valid_key(provider, ""), provider == Provider::Ollama);
        }
        assert_eq!(Provider::from_access_id("factory-openai-work"), None);
        assert_eq!(
            Provider::from_access_id("factory-google-not-a-generated-id"),
            None
        );
    }

    #[test]
    fn litellm_rejects_upstream_overrides_from_runners() {
        let mut body = json!({"model":"coding","messages":[],"tools":[],"stream":true});
        assert!(is_litellm_chat_request(&body));
        for key in [
            "api_base",
            "api_key",
            "fallbacks",
            "extra_headers",
            "custom_llm_provider",
        ] {
            body[key] = json!("override");
            assert!(!is_litellm_chat_request(&body));
            body.as_object_mut().unwrap().remove(key);
        }
    }
}
