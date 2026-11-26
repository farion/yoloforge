use super::{error, key_provider::read_json, login_context, private_json, Gateway};
use crate::{store::Store, App, Principal};
use axum::{
    extract::State,
    http::StatusCode,
    response::Response,
    Extension, Json,
};
use factory_core::RequestId;
use factory_protocol::models::{ModelAccess, OPENCODE_FREE_ACCESS};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashSet, time::Duration};

fn model_ids(payload: &Value) -> Vec<String> {
    let mut seen = HashSet::new();
    payload["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| {
            model["providerID"] == OPENCODE_FREE_ACCESS
                && model["enabled"] == true
                && model["type"] == "chat"
                && model["capabilities"]["tools"] == true
                && model["cost"].as_array().is_some_and(|tiers| {
                    !tiers.is_empty() && tiers.iter().all(|tier| {
                        tier["input"].as_f64() == Some(0.0)
                            && tier["output"].as_f64() == Some(0.0)
                            && ["read", "write"].iter().all(|key| {
                                tier["cache"].get(key).is_none_or(|cost| cost.as_f64() == Some(0.0))
                            })
                    })
                })
        })
        // Native selections use the catalog ID, not the upstream modelID.
        .filter_map(|model| model["id"].as_str())
        .filter(|id| crate::valid_text(id, 200) && seen.insert(*id))
        .take(256)
        .map(str::to_owned)
        .collect()
}

async fn discover(gateway: &Gateway) -> Option<Vec<String>> {
    let response = gateway.0.client
        .get(format!("{}/api/model", gateway.0.central.url.trim_end_matches('/')))
        .basic_auth("opencode", Some(&gateway.0.central.password))
        .timeout(Duration::from_secs(5))
        .send().await.ok()?.error_for_status().ok()?;
    let models = model_ids(&read_json(response).await?);
    (!models.is_empty()).then_some(models)
}

pub(super) async fn status(gateway: &Gateway, store: Store) -> &'static str {
    let Ok(Ok(settings)) = tokio::task::spawn_blocking(move || store.factory_models()).await else {
        return "unavailable";
    };
    let Some(access) = settings.accesses.iter().find(|a| a.id == OPENCODE_FREE_ACCESS) else {
        return "not_connected";
    };
    if discover(gateway).await.is_some_and(|models| access.models.iter().any(|id| models.contains(id))) {
        "ready"
    } else {
        "unavailable"
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Connect {
    name: String,
}

pub(crate) async fn connect(
    State(app): State<App>,
    Extension(principal): Extension<Principal>,
    gateway: Option<Extension<Gateway>>,
    Json(body): Json<Connect>,
) -> Response {
    let (gateway, store) = match login_context(app, principal, gateway) {
        Ok(context) => context,
        Err(response) => return response,
    };
    let name = body.name.trim().to_owned();
    if !crate::valid_text(&name, 120) {
        return error(StatusCode::BAD_REQUEST, "Ungültiger Name für den Modellzugang.");
    }
    let Some(models) = discover(&gateway).await else {
        return error(StatusCode::BAD_GATEWAY, "Kostenlose OpenCode-Modelle sind gerade nicht verfügbar.");
    };
    let access = ModelAccess { id: OPENCODE_FREE_ACCESS.into(), name, models, transports: Default::default() };
    match tokio::task::spawn_blocking(move || store.add_opencode_free_access(access)).await {
        Ok(Ok(default)) => private_json(json!({"connected":true,"automatic_default":default})),
        Ok(Err(err)) => crate::db_error(err, RequestId::new()),
        Err(_) => crate::db_error(crate::store::StoreError::Internal, RequestId::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_catalog_accepts_only_enabled_free_chat_models_with_tools() {
        let free = json!({"id":"big-pickle","modelID":"upstream-id","providerID":"opencode",
            "enabled":true,"type":"chat","capabilities":{"tools":true},"cost":[{"input":0,"output":0}]});
        let mut paid = free.clone();
        paid["cost"] = json!([{"input":0,"output":0},{"input":0,"output":1}]);
        let mut disabled = free.clone();
        disabled["enabled"] = false.into();
        let mut other = free.clone();
        other["providerID"] = "opencode-go".into();
        let mut tool_less = free.clone();
        tool_less["capabilities"]["tools"] = false.into();
        let mut unpriced = free.clone();
        unpriced["cost"] = json!([]);
        assert_eq!(model_ids(&json!({"data":[paid,disabled,other,tool_less,unpriced,free,free]})), ["big-pickle"]);
    }
}
