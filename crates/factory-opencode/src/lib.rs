//! Adapter pinned to the verified OpenCode 2.0.27 contract.
//! Credentials remain separate from Factory authentication and never implement Debug.
use anyhow::{ensure, Context, Result};
use factory_core::{OpenCodeStatus, QuestionKind, QuestionState};
use reqwest::{Client, Method, StatusCode, Url};
use serde::Deserialize;
use std::{fs, path::Path, time::Duration};

pub const SUPPORTED_VERSION: &str = "2.0.27";
mod excerpt;

#[derive(Deserialize)]
struct ServiceRegistration {
    url: String,
    password: String,
}

pub struct OpenCodeClient {
    client: Client,
    registration: ServiceRegistration,
}

impl OpenCodeClient {
    pub async fn compact(&self, session: &str, id: &str) -> Result<factory_core::CompactionState> {
        use factory_core::CompactionState;
        safe_id(session)?;
        safe_id(id)?;
        let response = self
            .client
            .post(format!(
                "{}/api/session/{session}/compact",
                self.registration.url.trim_end_matches('/')
            ))
            .basic_auth("opencode", Some(&self.registration.password))
            .json(&serde_json::json!({"id":id,"delivery":"steer"}))
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Compaction delivery uncertain"))?;
        if matches!(
            response.status(),
            StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND
        ) {
            return Ok(CompactionState::Rejected);
        }
        ensure!(
            response.status().is_success(),
            "Compaction admission uncertain"
        );
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|_| anyhow::anyhow!("Compaction admission response invalid"))?;
        ensure!(
            value["data"]["id"] == id
                && value["data"]["sessionID"] == session
                && value["data"]["type"] == "compaction",
            "Compaction admission identity differs"
        );
        Ok(CompactionState::Admitted)
    }
    pub async fn compaction_state(
        &self,
        session: &str,
        id: &str,
    ) -> Result<factory_core::CompactionState> {
        use factory_core::CompactionState;
        if let Some(message) = self.message(session, id).await? {
            ensure!(
                message["id"] == id
                    && message["type"] == "compaction"
                    && message["reason"] == "manual",
                "Compaction message identity differs"
            );
            return Ok(match message["status"].as_str() {
                Some("running") => CompactionState::Running,
                Some("completed") => CompactionState::Succeeded,
                Some("failed") => CompactionState::Failed,
                _ => anyhow::bail!("Unknown compaction message state"),
            });
        }
        let inbox = self.session_items(session, "inbox").await?;
        if let Some(item) = inbox
            .as_array()
            .context("Invalid compaction inbox")?
            .iter()
            .find(|v| v["id"] == id)
        {
            ensure!(
                item["type"] == "compaction" && item["sessionID"] == session,
                "Compaction inbox identity differs"
            );
            return Ok(CompactionState::Admitted);
        }
        Ok(CompactionState::Unknown)
    }
    /// Lossless projected message page, oldest first. Opaque cursors are not combined with order.
    pub async fn archive_page(
        &self,
        session: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<serde_json::Value>, Option<String>)> {
        safe_id(session)?;
        let mut request = self
            .client
            .get(format!(
                "{}/api/session/{session}/message",
                self.registration.url.trim_end_matches('/')
            ))
            .basic_auth("opencode", Some(&self.registration.password))
            .query(&[("limit", "16")]);
        request = if let Some(cursor) = cursor {
            request.query(&[("cursor", cursor)])
        } else {
            request.query(&[("order", "asc")])
        };
        let mut response = request.send().await?;
        ensure!(response.status().is_success(), "Archive page unavailable");
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 8 * 1024 * 1024 - 4096,
                "Archive page exceeds 8 MiB; no truncation applied"
            );
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let messages = value["data"]
            .as_array()
            .context("Archive messages missing")?
            .clone();
        ensure!(messages.len() <= 16, "Archive page limit exceeded");
        let cursors = value["cursor"]
            .as_object()
            .context("Archive cursor missing")?;
        let next = match cursors.get("next") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) if s.len() <= 4096 => Some(s.clone()),
            _ => anyhow::bail!("Invalid archive cursor"),
        };
        ensure!(
            next.as_deref() != cursor || next.is_none(),
            "Archive cursor did not advance"
        );
        Ok((messages, next))
    }
    /// Newest page only; return oldest-to-newest for display. Never interpret a failed read as empty.
    pub async fn excerpt(
        &self,
        session: &str,
    ) -> Result<(Vec<factory_core::ExcerptMessage>, bool)> {
        safe_id(session)?;
        let mut response = self
            .client
            .get(format!(
                "{}/api/session/{session}/message?order=desc&limit=32",
                self.registration.url.trim_end_matches('/')
            ))
            .basic_auth("opencode", Some(&self.registration.password))
            .send()
            .await?;
        ensure!(
            response.status().is_success(),
            "Session excerpt unavailable"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 8 * 1024 * 1024,
                "Session excerpt response too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        excerpt::project(serde_json::from_slice(&bytes)?)
    }
    pub async fn interrupt(&self, session: &str) -> Result<()> {
        safe_id(session)?;
        self.request(
            Method::POST,
            &format!("/api/session/{session}/interrupt?resume=false"),
            None,
        )
        .await?;
        Ok(())
    }
    pub async fn question_detail(
        &self,
        session: &str,
        kind: QuestionKind,
        id: &str,
    ) -> Result<Option<serde_json::Value>> {
        safe_id(session)?;
        safe_id(id)?;
        let resource = match kind {
            QuestionKind::Form => "form",
            QuestionKind::Permission => "permission",
        };
        Ok(self
            .request(
                Method::GET,
                &format!("/api/session/{session}/{resource}/{id}"),
                None,
            )
            .await?
            .map(|v| v["data"].clone()))
    }
    pub async fn reply_question(
        &self,
        session: &str,
        kind: QuestionKind,
        id: &str,
        answer: &serde_json::Value,
    ) -> Result<QuestionState> {
        safe_id(session)?;
        safe_id(id)?;
        let resource = match kind {
            QuestionKind::Form => "form",
            QuestionKind::Permission => "permission",
        };
        let body = if kind == QuestionKind::Form {
            serde_json::json!({"answer":answer})
        } else {
            answer.clone()
        };
        let response = self
            .client
            .post(format!(
                "{}/api/session/{session}/{resource}/{id}/reply",
                self.registration.url.trim_end_matches('/')
            ))
            .basic_auth("opencode", Some(&self.registration.password))
            .json(&body)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Question reply transport uncertain"))?;
        Ok(if response.status().is_success() {
            QuestionState::Answered
        } else if response.status() == StatusCode::BAD_REQUEST {
            QuestionState::Rejected
        } else {
            QuestionState::Unknown
        })
    }
    pub fn endpoint(&self) -> &str {
        &self.registration.url
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>> {
        let mut request = self
            .client
            .request(
                method,
                format!("{}{}", self.registration.url.trim_end_matches('/'), path),
            )
            .basic_auth("opencode", Some(&self.registration.password));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("OpenCode transport unavailable"))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        ensure!(
            response.status().is_success(),
            "OpenCode HTTP {}",
            response.status().as_u16()
        );
        Ok(Some(response.json().await.map_err(|_| {
            anyhow::anyhow!("Invalid OpenCode response")
        })?))
    }
    pub async fn session(&self, id: &str) -> Result<Option<serde_json::Value>> {
        safe_id(id)?;
        Ok(self
            .request(Method::GET, &format!("/api/session/{id}"), None)
            .await?
            .map(|v| v["data"].clone()))
    }
    pub async fn create_session(
        &self,
        id: &str,
        directory: &str,
        command: &str,
        provider: &str,
        model: &str,
    ) -> Result<()> {
        safe_id(id)?;
        let result = self
            .request(
                Method::POST,
                "/api/session",
                Some(serde_json::json!({
                    "id":id,"title":format!("Factory {command}"),"agent":"build",
                    "location":{"directory":directory},"model":{"providerID":provider,"id":model},
                    "metadata":{"factory_command":command}
                })),
            )
            .await?;
        ensure!(
            result
                .as_ref()
                .and_then(|v| v.pointer("/data/id"))
                .and_then(|v| v.as_str())
                == Some(id),
            "Session creation not confirmed"
        );
        Ok(())
    }
    pub async fn prompt(&self, session: &str, id: &str, text: &str) -> Result<()> {
        safe_id(session)?;
        safe_id(id)?;
        let value = self
            .request(
                Method::POST,
                &format!("/api/session/{session}/prompt"),
                Some(serde_json::json!({"id":id,"text":text})),
            )
            .await?;
        ensure!(
            value
                .as_ref()
                .and_then(|v| v.pointer("/data/id"))
                .and_then(|v| v.as_str())
                == Some(id),
            "Prompt admission not confirmed"
        );
        ensure!(
            value
                .as_ref()
                .and_then(|v| v.pointer("/data/payload/text"))
                .and_then(|v| v.as_str())
                == Some(text),
            "Prompt payload differs"
        );
        Ok(())
    }
    pub async fn message(&self, session: &str, id: &str) -> Result<Option<serde_json::Value>> {
        safe_id(session)?;
        safe_id(id)?;
        Ok(self
            .request(
                Method::GET,
                &format!("/api/session/{session}/message/{id}"),
                None,
            )
            .await?
            .map(|v| v["data"].clone()))
    }
    pub async fn session_items(&self, session: &str, kind: &str) -> Result<serde_json::Value> {
        safe_id(session)?;
        ensure!(
            matches!(
                kind,
                "inbox" | "form" | "permission" | "message?type=assistant&order=desc&limit=1"
            ),
            "Invalid session resource"
        );
        let value = self
            .request(Method::GET, &format!("/api/session/{session}/{kind}"), None)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session resource missing"))?;
        ensure!(
            value["data"].is_array(),
            "Invalid session resource response"
        );
        Ok(value["data"].clone())
    }
    pub async fn active(&self, session: &str) -> Result<bool> {
        let value = self
            .request(Method::GET, "/api/session/active", None)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Active sessions unavailable"))?;
        let data = value["data"]
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Invalid active sessions response"))?;
        Ok(data.contains_key(session))
    }
    pub fn from_registration(path: &Path) -> Result<Self> {
        let registration: ServiceRegistration = serde_json::from_slice(&fs::read(path)?)?;
        let url = Url::parse(&registration.url)?;
        ensure!(
            url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                && url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none(),
            "OpenCode endpoint must be loopback HTTP"
        );
        ensure!(
            !registration.password.is_empty(),
            "OpenCode password is empty"
        );
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(3))
                .build()?,
            registration,
        })
    }

    pub async fn status(&self) -> OpenCodeStatus {
        #[derive(Deserialize)]
        struct Info {
            version: String,
        }
        let response = self
            .client
            .get(format!(
                "{}/api/info",
                self.registration.url.trim_end_matches('/')
            ))
            .basic_auth("opencode", Some(&self.registration.password))
            .send()
            .await;
        let Ok(response) = response else {
            return OpenCodeStatus::Unavailable;
        };
        if !response.status().is_success() {
            return OpenCodeStatus::Unavailable;
        }
        let Ok(info) = response.json::<Info>().await else {
            return OpenCodeStatus::Unavailable;
        };
        if info.version == SUPPORTED_VERSION {
            OpenCodeStatus::Ready {
                version: info.version,
            }
        } else {
            OpenCodeStatus::Unsupported {
                version: info.version,
            }
        }
    }
}

fn safe_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() < 200
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "Invalid OpenCode ID"
    );
    Ok(())
}
