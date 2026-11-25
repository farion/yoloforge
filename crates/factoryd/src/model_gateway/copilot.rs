use super::{error, Entry, Gateway, Store};
use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use factory_core::RequestId;
use factory_protocol::models::{is_copilot_access, ModelTransport, COPILOT_INTEGRATION};
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};
use url::Url;

const DEFAULT_ENDPOINT: &str = "https://api.githubcopilot.com";

pub(super) struct Credential {
    token: String,
    endpoint: String,
}

fn endpoint(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    let host = url.host_str()?;
    (url.scheme() == "https"
        && host.ends_with(".githubcopilot.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none())
    .then(|| url.as_str().trim_end_matches('/').to_owned())
}

fn from_entry(entry: &Entry) -> Option<Credential> {
    if entry.integration_id != COPILOT_INTEGRATION
        || entry.value["type"] != "oauth"
        || entry.value["methodID"] != "device"
        || entry.value["metadata"]["enterpriseUrl"].is_string()
    {
        return None;
    }
    // OpenCode stores the public GitHub device token in both OAuth fields with
    // expires=0, unlike the short-lived ChatGPT credential.
    let expires = entry.value["expires"].as_u64()?;
    if expires != 0 && expires <= crate::unix_ms() {
        return None;
    }
    let token = entry.value["refresh"].as_str()?;
    if token.is_empty() || token.len() > 4096 || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    let endpoint = match entry.value["metadata"]["apiEndpoint"].as_str() {
        Some(value) => endpoint(value)?,
        None => DEFAULT_ENDPOINT.into(),
    };
    Some(Credential {
        token: token.into(),
        endpoint,
    })
}

pub(super) async fn credential(gateway: &Gateway, id: &str) -> Option<Credential> {
    let entry = gateway
        .entries()
        .await?
        .into_iter()
        .find(|entry| entry.id == id)?;
    from_entry(&entry)
}

fn user_agent() -> String {
    format!(
        "opencode/stable/{}/server",
        factory_opencode::distribution::VERSION
    )
}

pub(super) async fn account(gateway: &Gateway, entry: &Entry) -> Option<String> {
    let credential = from_entry(entry)?;
    let response = gateway
        .0
        .client
        .get(format!(
            "{}/user",
            request_base(gateway, "https://api.github.com")
        ))
        .bearer_auth(credential.token)
        .header(header::USER_AGENT, user_agent())
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(6))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let body = super::key_provider::read_json(response).await?;
    body["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .map(|id| id.to_string())
}

fn transport(id: &str, endpoints: &Value) -> ModelTransport {
    let supports = |path| {
        endpoints
            .as_array()
            .is_some_and(|items| items.iter().any(|v| v == path))
    };
    if supports("/v1/messages") {
        ModelTransport::Messages
    } else if supports("/responses") {
        ModelTransport::Responses
    } else if supports("/chat/completions") {
        ModelTransport::Chat
    } else if id.starts_with("grok-")
        || id.starts_with("mai-code-")
        || (!id.starts_with("gpt-5-mini")
            && id
                .strip_prefix("gpt-")
                .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                .and_then(|n| n.parse::<u32>().ok())
                .is_some_and(|n| n >= 5))
    {
        ModelTransport::Responses
    } else {
        ModelTransport::Chat
    }
}

type Catalog = (Vec<String>, BTreeMap<String, ModelTransport>);

fn collect_models(body: &Value) -> Option<Catalog> {
    let mut models = Vec::new();
    let mut transports = BTreeMap::new();
    for model in body["data"].as_array()?.iter().filter(|m| {
        m["policy"]["state"] != "disabled"
            && m["capabilities"]["supports"]["tool_calls"] == true
            && m["capabilities"]["limits"]["max_output_tokens"]
                .as_u64()
                .is_some()
            && m["capabilities"]["limits"]["max_prompt_tokens"]
                .as_u64()
                .is_some()
    }) {
        let Some(id) = model["id"].as_str().filter(|id| {
            crate::valid_text(id, 200) && !matches!(*id, "auto" | "gpt-5-chat-latest")
        }) else {
            continue;
        };
        if transports
            .insert(id.into(), transport(id, &model["supported_endpoints"]))
            .is_none()
        {
            models.push(id.into());
        }
        if models.len() >= 256 {
            break;
        }
    }
    (!models.is_empty()).then_some((models, transports))
}

pub(super) async fn models(gateway: &Gateway, entry: &Entry) -> Option<Catalog> {
    let credential = from_entry(entry)?;
    let response = github_request(
        gateway.0.client.get(format!(
            "{}/models",
            request_base(gateway, &credential.endpoint)
        )),
        &credential,
    )
    .timeout(Duration::from_secs(8))
    .send()
    .await
    .ok()?
    .error_for_status()
    .ok()?;
    collect_models(&super::key_provider::read_json(response).await?)
}

fn github_request(
    request: reqwest::RequestBuilder,
    credential: &Credential,
) -> reqwest::RequestBuilder {
    request
        .bearer_auth(&credential.token)
        .header(header::USER_AGENT, user_agent())
        .header("X-GitHub-Api-Version", "2026-08-01")
}

fn request_base<'a>(_gateway: &'a Gateway, base: &'a str) -> &'a str {
    #[cfg(test)]
    if let Some(upstream) = &_gateway.0.copilot_test_upstream {
        return upstream;
    }
    base
}

pub(super) async fn relay(
    (gateway, store): (Gateway, Store),
    alias: String,
    mut headers: HeaderMap,
    mut body: Value,
    transport: ModelTransport,
) -> Response {
    if transport == ModelTransport::Messages && !headers.contains_key(header::AUTHORIZATION) {
        if let Some(auth) = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .and_then(|key| HeaderValue::from_str(&format!("Bearer {key}")).ok())
        {
            headers.insert(header::AUTHORIZATION, auth);
        }
    }
    if !gateway.allows_runner(&headers, &store).await {
        return error(StatusCode::UNAUTHORIZED, "Factory model key required");
    }
    if !is_copilot_access(&alias) {
        return error(StatusCode::NOT_FOUND, "Factory model access unavailable");
    }
    let Some(payload) = body.as_object_mut() else {
        return error(StatusCode::BAD_REQUEST, "Invalid Copilot request");
    };
    let Some(model) = payload
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return error(StatusCode::BAD_REQUEST, "Model required");
    };
    if payload.contains_key("previous_response_id")
        || payload.contains_key("conversation")
        || payload.contains_key("models")
    {
        return error(
            StatusCode::BAD_REQUEST,
            "Stateful or alternate model requests are not supported",
        );
    }
    if transport == ModelTransport::Responses {
        payload.insert("store".into(), Value::Bool(false));
    }
    let catalog = store.clone();
    let access = alias.clone();
    let Ok(Ok(settings)) = tokio::task::spawn_blocking(move || catalog.factory_models()).await
    else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Factory model catalog unavailable",
        );
    };
    if !settings.accesses.iter().any(|a| {
        a.id == access && a.models.contains(&model) && a.transports.get(&model) == Some(&transport)
    }) {
        return error(
            StatusCode::FORBIDDEN,
            "Model or transport not permitted for this access",
        );
    }
    let Some(id) = gateway.binding(store, alias).await else {
        return error(StatusCode::NOT_FOUND, "Factory model access unavailable");
    };
    let Some(credential) = credential(&gateway, &id).await else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHub Copilot access is not connected",
        );
    };
    let path = match transport {
        ModelTransport::Chat => "/chat/completions",
        ModelTransport::Responses => "/responses",
        ModelTransport::Messages => "/v1/messages",
    };
    let last_role = body[if transport == ModelTransport::Responses {
        "input"
    } else {
        "messages"
    }]
    .as_array()
    .and_then(|items| items.last())
    .and_then(|item| item["role"].as_str());
    let mut request = github_request(
        gateway.0.client.post(format!(
            "{}{path}",
            request_base(&gateway, &credential.endpoint)
        )),
        &credential,
    )
    .header("Openai-Intent", "conversation-edits")
    .header("X-Interaction-Id", RequestId::new().to_string())
    .header("X-Interaction-Type", "conversation-agent")
    .header(
        "x-initiator",
        if last_role == Some("user") {
            "user"
        } else {
            "agent"
        },
    );
    if transport == ModelTransport::Messages {
        request = request.header("anthropic-version", "2023-06-01");
    }
    let response = request.json(&body).send().await;
    match response {
        Ok(response) if response.status().is_success() => {
            let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
            let mut result = Body::from_stream(response.bytes_stream()).into_response();
            if let Some(value) = content_type {
                result.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            result
        }
        Ok(response) if response.status() == StatusCode::TOO_MANY_REQUESTS => error(
            StatusCode::TOO_MANY_REQUESTS,
            "GitHub Copilot request limit reached",
        ),
        Ok(response) => error(
            StatusCode::BAD_GATEWAY,
            &format!(
                "GitHub Copilot rejected the request (HTTP {})",
                response.status().as_u16()
            ),
        ),
        Err(_) => error(StatusCode::BAD_GATEWAY, "GitHub Copilot unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn oauth_uses_the_public_copilot_endpoint_and_accepts_non_expiring_device_credentials() {
        let mut entry = Entry {
            id: "test".into(),
            label: "test".into(),
            integration_id: COPILOT_INTEGRATION.into(),
            active: true,
            value: json!({"type":"oauth","methodID":"device","refresh":"test-token","expires":0}),
        };
        assert_eq!(from_entry(&entry).unwrap().endpoint, DEFAULT_ENDPOINT);
        entry.value["metadata"] = json!({"apiEndpoint":"https://api.business.githubcopilot.com"});
        assert!(from_entry(&entry).is_some());
        for invalid in [
            "https://githubcopilot.com.example.com",
            "http://api.githubcopilot.com",
            "https://api.githubcopilot.com:444",
            "https://api.githubcopilot.com/other",
        ] {
            entry.value["metadata"]["apiEndpoint"] = json!(invalid);
            assert!(from_entry(&entry).is_none(), "{invalid}");
        }
    }

    #[test]
    fn catalog_keeps_allowed_tool_models_and_their_native_protocols() {
        let model = |id: &str, path: &str, tools: bool| {
            json!({
                "id":id,"supported_endpoints":[path],
                "capabilities":{"supports":{"tool_calls":tools},"limits":{"max_output_tokens":1,"max_prompt_tokens":2}}
            })
        };
        let (models, transports) = collect_models(&json!({"data":[
            model("gpt-5","/responses",true), model("claude-sonnet","/v1/messages",true),
            model("gpt-4o","/chat/completions",true), model("plain","/chat/completions",false)
        ]}))
        .unwrap();
        assert_eq!(models, ["gpt-5", "claude-sonnet", "gpt-4o"]);
        assert_eq!(transports["gpt-5"], ModelTransport::Responses);
        assert_eq!(transports["claude-sonnet"], ModelTransport::Messages);
        assert_eq!(transports["gpt-4o"], ModelTransport::Chat);
    }
}
