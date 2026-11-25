//! Adapter for the native OpenCode V2 HTTP API.
//! Credentials remain separate from Factory authentication and never implement Debug.
use anyhow::{ensure, Context, Result};
use factory_core::{OpenCodeStatus, QuestionKind, QuestionState};
use reqwest::{Client, Method, StatusCode, Url};
use serde::Deserialize;
use std::{fs, path::Path, time::Duration};

pub mod distribution;

fn supported_version(version: &str) -> bool {
    let mut parts = version.trim().split(['-', '+']).next().unwrap_or("").split('.');
    matches!(
        (parts.next(), parts.next().and_then(|v| v.parse::<u32>().ok()),
         parts.next().and_then(|v| v.parse::<u32>().ok()), parts.next()),
        (Some("2"), Some(minor), Some(patch), None) if minor > 0 || patch >= 27
    )
}

#[test]
fn native_v2_versions_are_not_pinned_to_one_patch() {
    for version in ["2.0.27", "2.0.99", "2.4.1", "2.9.0+build"] {
        assert!(supported_version(version), "{version}");
    }
    for version in ["1.4.1", "2.0.26", "3.0.0", "2.1", "not-a-version"] {
        assert!(!supported_version(version), "{version}");
    }
}
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
    /// A local, user-facing entry into OpenCode's provider and MCP sign-in UI.
    pub async fn project_terminal_command(
        &self,
        directory: &Path,
    ) -> Result<std::process::Command> {
        ensure!(
            matches!(self.status().await, OpenCodeStatus::Ready { .. }),
            "OpenCode-Service nicht erreichbar oder inkompatibel"
        );
        let directory = fs::canonicalize(directory)?;
        ensure!(directory.is_dir(), "Projektarbeitsordner fehlt");
        let mut command = std::process::Command::new("opencode");
        command
            .args(["--server", self.registration.url.as_str()])
            .arg(&directory)
            .current_dir(directory)
            .env("OPENCODE_PASSWORD", &self.registration.password)
            .env_remove("FACTORY_TOKEN")
            .env_remove("FACTORY_TOKEN_FILE");
        Ok(command)
    }
    /// Read-only status, deliberately not a paid model probe or credential export.
    pub async fn readiness(&self, directory: &str, provider: &str, model: &str) -> Result<String> {
        ensure!(
            matches!(self.status().await, OpenCodeStatus::Ready { .. }),
            "OpenCode-Service nicht erreichbar oder inkompatibel"
        );
        let mut lines = vec!["OpenCode-V2-Service erreichbar".to_string()];
        if provider.is_empty() || model.is_empty() {
            lines.push("Projektmodell fehlt: Provider-ID und Modell-ID explizit konfigurieren. Kein automatisches Ersatzmodell.".into());
        } else {
            lines.push(format!("Projektmodell: {provider}/{model}"));
        }
        for (path, label) in [("/api/integration", "Anmeldungen"), ("/api/mcp", "MCP")] {
            let value = self.location_get(path, directory).await?;
            let items = value
                .as_array()
                .context("Invalid OpenCode status response")?;
            if items.is_empty() {
                lines.push(format!("{label}: keine Eintraege"));
            }
            for item in items {
                if path == "/api/mcp" {
                    let name = item["name"].as_str().unwrap_or("?");
                    let status = item["status"]["status"].as_str().unwrap_or("unknown");
                    lines.push(format!(
                        "MCP {name}: {status}{}",
                        if status == "needs_auth" {
                            " — in OpenCode /mcps anmelden"
                        } else {
                            ""
                        }
                    ));
                } else {
                    let name = item["name"].as_str().unwrap_or("?");
                    let count = item["connections"].as_array().map_or(0, Vec::len);
                    if count == 0 && !item["id"].as_str().is_some_and(|id| id == provider) {
                        continue;
                    }
                    let needs_auth = item["connections"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|c| c["status"]["status"] == "needs_auth"));
                    lines.push(format!(
                        "Anmeldung {name}: {}",
                        if needs_auth {
                            "Anmeldung abgelaufen oder erforderlich — OpenCode oeffnen"
                        } else if count == 0 {
                            "keine Verbindung hinterlegt"
                        } else {
                            "Verbindung hinterlegt (kein Modellaufruf getestet)"
                        }
                    ));
                }
            }
        }
        lines.push("\nOAuth/Provider und MCP-Anmeldung: Projektliste → o → OpenCode-Anmeldedialoge.\nStatus erneut mit r pruefen. Hinterlegte Verbindungen beweisen keine gueltigen Tokens oder Modellverfuegbarkeit.".into());
        Ok(lines
            .join("\n")
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .collect())
    }
    async fn location_get(&self, path: &str, directory: &str) -> Result<serde_json::Value> {
        let response = self
            .client
            .get(format!(
                "{}{}",
                self.registration.url.trim_end_matches('/'),
                path
            ))
            .basic_auth("opencode", Some(&self.registration.password))
            .query(&[("location[directory]", directory)])
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("OpenCode-Verbindung nicht verfuegbar"))?;
        ensure!(
            response.status().is_success(),
            "OpenCode-Status HTTP {}",
            response.status().as_u16()
        );
        let value: serde_json::Value = response
            .json()
            .await
            .context("Invalid OpenCode status response")?;
        ensure!(
            value["location"]["directory"] == directory,
            "OpenCode-Status gehoert zu anderem Projektordner"
        );
        Ok(value["data"].clone())
    }
    pub async fn default_model(&self, directory: &str) -> Result<(String, String)> {
        let response = self
            .client
            .get(format!(
                "{}/api/model/default",
                self.registration.url.trim_end_matches('/')
            ))
            .basic_auth("opencode", Some(&self.registration.password))
            .query(&[("location[directory]", directory)])
            .send()
            .await?;
        ensure!(
            response.status().is_success(),
            "OpenCode default model unavailable"
        );
        let value: serde_json::Value = response.json().await?;
        ensure!(
            value["location"]["directory"] == directory,
            "Default model belongs to another location"
        );
        let model = &value["data"];
        let provider = model["providerID"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("No default provider; OpenCode sign-in required")?;
        let id = model["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("No default model; OpenCode sign-in required")?;
        Ok((provider.into(), id.into()))
    }
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
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(Some(serde_json::Value::Null));
        }
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
    /// Open the original terminal client on an existing, location-checked session.
    /// Credentials are inherited through the child environment, never command-line arguments.
    pub async fn terminal_command(
        &self,
        id: &str,
        directory: &Path,
    ) -> Result<std::process::Command> {
        ensure!(
            matches!(self.status().await, OpenCodeStatus::Ready { .. }),
            "OpenCode-Service nicht erreichbar oder inkompatibel"
        );
        let session = self.session(id).await?.context(
            "Session existiert in diesem OpenCode-Service nicht; keine neue Session angelegt",
        )?;
        ensure!(
            session["id"].as_str() == Some(id),
            "OpenCode returned a different session"
        );
        let actual = session["location"]["directory"]
            .as_str()
            .context("Session location missing")?;
        ensure!(
            fs::canonicalize(actual)? == fs::canonicalize(directory)?,
            "Session und Arbeitsordner stimmen nicht ueberein"
        );
        let mut command = std::process::Command::new("opencode");
        command
            .args(["--server", self.registration.url.as_str(), "--session", id])
            .arg(directory)
            .current_dir(directory)
            .env("OPENCODE_PASSWORD", &self.registration.password)
            .env_remove("FACTORY_TOKEN")
            .env_remove("FACTORY_TOKEN_FILE");
        Ok(command)
    }
    pub async fn create_session(
        &self,
        id: &str,
        directory: &str,
        command: &str,
        provider: &str,
        model: &str,
        permissions: Option<serde_json::Value>,
        direct: bool,
    ) -> Result<()> {
        safe_id(id)?;
        let mut metadata = serde_json::json!({"factory_command":command});
        if direct {
            metadata["factory_direct"] = serde_json::Value::Bool(true);
        }
        let result = self
            .request(
                Method::POST,
                "/api/session",
                Some(serde_json::json!({
                    "id":id,"agent":"build",
                    "location":{"directory":directory},"model":{"providerID":provider,"id":model},
                    "metadata":metadata,
                    "permissions": permissions
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

    pub async fn repo_skill_paths(&self, directory: &str) -> Result<serde_json::Value> {
        self.location_get("/api/skill", directory).await
    }

    pub async fn set_session_permissions(&self, id: &str, permissions: &serde_json::Value) -> Result<()> {
        safe_id(id)?;
        let updated = self.request(Method::PATCH, &format!("/api/session/{id}"),
            Some(serde_json::json!({"permissions":permissions}))).await?;
        ensure!(updated.as_ref().and_then(|r| r.pointer("/data/permissions")) == Some(permissions),
            "OpenCode did not confirm the session permissions");
        Ok(())
    }

    pub async fn local_mcp_statuses(&self, directory: &str) -> Result<std::collections::BTreeMap<String, String>> {
        let data = self.location_get("/api/mcp", directory).await?;
        data.as_array().context("MCP status list missing")?.iter().map(|entry| {
            Ok((
                entry["name"].as_str().context("MCP name missing")?.to_owned(),
                entry["status"]["status"].as_str().context("MCP status missing")?.to_owned(),
            ))
        }).collect()
    }

    pub async fn connect_local_mcp(&self, directory: &str, name: &str, command: &[String]) -> Result<()> {
        self.connect_mcp(directory, name, serde_json::json!({"type":"local","command":command,
            "codemode":false,"pool":false,"timeout":{"startup":5000,"catalog":5000}})).await
    }

    pub async fn connect_remote_mcp(&self, directory: &str, name: &str, url: &str, token: &str) -> Result<()> {
        self.connect_mcp(directory, name, serde_json::json!({"type":"remote","url":url,
            "headers":{"Authorization":format!("Bearer {token}")},"oauth":false,
            "codemode":false,"pool":false,"timeout":{"startup":8000,"catalog":8000}})).await
    }

    pub async fn remove_local_mcp_probe(&self, directory: &str) {
        // Fixed name; this endpoint is used only in a fresh, private probe location.
        let _ = self.client.delete(format!(
            "{}/api/experimental/mcp/yf-local-probe", self.registration.url.trim_end_matches('/')
        )).basic_auth("opencode", Some(&self.registration.password))
            .query(&[("location[directory]", directory)])
            .timeout(std::time::Duration::from_secs(4)).send().await;
    }

    async fn connect_mcp(&self, directory: &str, name: &str, config: serde_json::Value) -> Result<()> {
        ensure!(!name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'), "Invalid MCP name");
        let response = self.client.put(format!(
            "{}/api/experimental/mcp/{name}", self.registration.url.trim_end_matches('/')
        )).basic_auth("opencode", Some(&self.registration.password))
            .query(&[("location[directory]", directory)])
            .json(&serde_json::json!({"config":config}))
            .send().await.map_err(|_| anyhow::anyhow!("OpenCode MCP connection unavailable"))?;
        ensure!(response.status() == reqwest::StatusCode::NO_CONTENT, "OpenCode did not accept MCP");
        Ok(())
    }
    /// Archive without deleting history. Repeating this update is a native no-op.
    pub async fn archive_session(&self, session: &str) -> Result<()> {
        safe_id(session)?;
        let updated = self
            .request(
                Method::PATCH,
                &format!("/api/session/{session}"),
                Some(serde_json::json!({"archived": true})),
            )
            .await?;
        ensure!(
            updated.is_some(),
            "Decision session disappeared during archival"
        );
        Ok(())
    }
    pub async fn prompt(&self, session: &str, id: &str, text: &str) -> Result<()> {
        self.prompt_with_images(session, id, text, &[]).await
    }
    pub async fn prompt_with_images(
        &self,
        session: &str,
        id: &str,
        text: &str,
        images: &[factory_core::PromptImage],
    ) -> Result<()> {
        self.prompt_with_files(session, id, text, images, &[]).await
    }
    pub async fn prompt_with_files(
        &self,
        session: &str,
        id: &str,
        text: &str,
        images: &[factory_core::PromptImage],
        files: &[factory_core::PromptFile],
    ) -> Result<()> {
        safe_id(session)?;
        safe_id(id)?;
        factory_core::validate_images(images).map_err(anyhow::Error::msg)?;
        factory_core::validate_files(files).map_err(anyhow::Error::msg)?;
        let mut body = serde_json::json!({"id":id,"text":text});
        if !images.is_empty() || !files.is_empty() {
            body["files"] = serde_json::json!(images
                .iter()
                .map(|i| serde_json::json!({"uri":i.uri(),"name":i.name}))
                .chain(
                    files
                        .iter()
                        .map(|f| serde_json::json!({"uri":f.uri(),"name":f.name}))
                )
                .collect::<Vec<_>>());
            body["metadata"] =
                serde_json::json!({"factory_images":factory_core::image_digest(images)});
            if !files.is_empty() {
                body["metadata"]["factory_files"] =
                    serde_json::json!(factory_core::files_digest(files));
            }
        }
        let value = self
            .request(
                Method::POST,
                &format!("/api/session/{session}/prompt"),
                Some(body),
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
        if supported_version(&info.version) {
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

pub fn session_permissions(read_only: bool, repo_skills: Option<&[String]>, factory_skills: &[String], local_mcps: &[String], managed: bool) -> Option<serde_json::Value> {
    if read_only {
        return Some(serde_json::json!([{"action":"*","resource":"*","effect":"deny"}]));
    }
    let mut rules = Vec::new();
    if let Some(skills) = repo_skills {
        rules.push(serde_json::json!({"action":"skill","resource":"*","effect":"deny"}));
        rules.extend(skills.iter().map(|id| serde_json::json!({"action":"skill","resource":id,"effect":"allow"})));
    }
    if managed {
        rules.push(serde_json::json!({"action":"skill","resource":format!("{}*",factory_core::FACTORY_SKILL_PREFIX),"effect":"deny"}));
        rules.extend(factory_skills.iter().map(|id| serde_json::json!({"action":"skill","resource":id,"effect":"allow"})));
    }
    for name in local_mcps {
        rules.push(serde_json::json!({"action":format!("{name}_*"),"resource":"*","effect":"allow"}));
        rules.push(serde_json::json!({"action":"opencode_list_mcp_resources","resource":name,"effect":"allow"}));
        rules.push(serde_json::json!({"action":"opencode_read_mcp_resource","resource":format!("{name}:*"),"effect":"allow"}));
    }
    (!rules.is_empty()).then_some(serde_json::Value::Array(rules))
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
