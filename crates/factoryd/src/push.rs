//! Optional ntfy companion-app push. No Factory credentials or task content leave the service.
use crate::store::Store;
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    endpoint: String,
    #[serde(default)]
    token_file: Option<PathBuf>,
}
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = serde_json::from_slice(
            &std::fs::read(path).context("Push configuration not readable")?,
        )
        .context("Invalid push configuration")?;
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.endpoint)
            .map_err(|_| anyhow::anyhow!("Invalid ntfy endpoint"))?;
        let local = url
            .host_str()
            .is_some_and(|s| matches!(s, "127.0.0.1" | "[::1]" | "localhost"));
        ensure!(
            (url.scheme() == "https" || url.scheme() == "http" && local)
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "ntfy requires HTTPS without credentials (HTTP only on loopback)"
        );
        let topic = url.path().rsplit('/').next().unwrap_or("");
        ensure!(
            !topic.is_empty()
                && topic.len() <= 64
                && topic
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
            "ntfy endpoint must end with a topic name"
        );
        Ok(())
    }
    fn identity(&self) -> String {
        format!("{:x}", Sha256::digest(self.endpoint.as_bytes()))
    }
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
async fn deliver(client: &reqwest::Client, config: &Config) -> Result<()> {
    let mut request=client.post(&config.endpoint).header("Title","YoloForge: Rueckfragen").header("Click","yoloforge://questions").header("Priority","default").header("Cache","yes").body("In YoloForge warten Rueckfragen auf eine Antwort. Zum aktuellen Stand die App oeffnen.");
    if let Some(path) = &config.token_file {
        let token = std::fs::read_to_string(path).context("Push token file unavailable")?;
        let token = token.trim();
        ensure!(
            !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic()),
            "Invalid push token file"
        );
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Push transport failed"))?;
    ensure!(
        response.status().is_success(),
        "Push provider did not accept delivery"
    );
    Ok(())
}
pub async fn run(store: Store, config: Config, stopping: Arc<AtomicBool>) {
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            eprintln!("Push client initialization failed");
            return;
        }
    };
    let destination = config.identity();
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        interval.tick().await;
        if stopping.load(Ordering::Relaxed) {
            return;
        }
        let db = store.clone();
        let bound = destination.clone();
        let sequence = match tokio::task::spawn_blocking(move || db.claim_push(&bound, now())).await
        {
            Ok(Ok(Some(sequence))) => sequence,
            Ok(Ok(None)) => continue,
            _ => {
                eprintln!("Push queue unavailable; retry retained");
                continue;
            }
        };
        if deliver(&client, &config).await.is_err() {
            eprintln!("Push delivery unconfirmed; durable retry scheduled");
            continue;
        }
        let db = store.clone();
        let bound = destination.clone();
        if !matches!(
            tokio::task::spawn_blocking(move || db.confirm_push(&bound, sequence, now())).await,
            Ok(Ok(()))
        ) {
            eprintln!("Push confirmation not persisted; duplicate notification possible");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn push_has_no_factory_data_or_tokens_and_does_not_follow_redirects() {
        use axum::{
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
            routing::post,
            Router,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/topic", listener.local_addr().unwrap());
        let router = Router::new().route(
            "/topic",
            post(|headers: HeaderMap, body: String| async move {
                assert_eq!(headers["click"], "yoloforge://questions");
                assert!(!headers.contains_key("authorization"));
                assert!(!body.contains("question_id"));
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [("location", "http://127.0.0.1:1/secret")],
                )
                    .into_response()
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let config = Config {
            endpoint,
            token_file: None,
        };
        config.validate().unwrap();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        assert!(deliver(&client, &config).await.is_err());
        server.abort();
        for endpoint in [
            "http://remote.example/topic",
            "https://user:pass@host/topic",
            "https://host/topic?token=secret",
        ] {
            assert!(Config {
                endpoint: endpoint.into(),
                token_file: None
            }
            .validate()
            .is_err());
        }
    }
}
