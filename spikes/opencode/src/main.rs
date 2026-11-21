//! Narrow, version-pinned integration probe. Never prints credentials or response bodies.
use anyhow::{bail, ensure, Context, Result};
use reqwest::{Client, Method, Response, StatusCode, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{env, fs, path::PathBuf, time::Duration};

#[derive(Deserialize)]
struct Registration {
    url: String,
    password: String,
}

struct Api {
    client: Client,
    registration: Registration,
}

impl Api {
    fn new(registration: Registration) -> Result<Self> {
        let url = Url::parse(&registration.url)?;
        // This probe only accepts a local service or an already established SSH tunnel.
        ensure!(
            url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path() == "/",
            "registration must point to a loopback HTTP origin"
        );
        Ok(Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .connect_timeout(Duration::from_secs(5))
                .build()?,
            registration,
        })
    }

    async fn response(&self, method: Method, path: &str, body: Option<Value>) -> Result<Response> {
        let mut request = self
            .client
            .request(
                method,
                format!("{}{}", self.registration.url.trim_end_matches('/'), path),
            )
            .basic_auth("opencode", Some(&self.registration.password))
            .timeout(Duration::from_secs(20));
        if let Some(body) = body {
            request = request.json(&body);
        }
        request.send().await.context("OpenCode request failed")
    }

    async fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let response = self.response(method, path, body).await?;
        ensure!(
            response.status().is_success(),
            "OpenCode returned HTTP {}",
            response.status()
        );
        let bytes = response.bytes().await.context("cannot read response")?;
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes).context("invalid JSON response")
    }

    async fn get(&self, path: &str) -> Result<Value> {
        self.request(Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.request(Method::POST, path, Some(body)).await
    }

    async fn events(&self) -> Result<Response> {
        let response = self.response(Method::GET, "/api/event", None).await?;
        ensure!(response.status().is_success(), "event stream rejected");
        ensure!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("text/event-stream")),
            "expected SSE"
        );
        Ok(response)
    }
}

// Bounded SSE framing: only collect complete events, tolerate arbitrary chunk boundaries.
#[derive(Default)]
struct Frames {
    buffer: String,
}
impl Frames {
    fn push(&mut self, text: &str) -> Result<Vec<String>> {
        self.buffer.push_str(text);
        ensure!(self.buffer.len() < 1_048_576, "SSE frame limit exceeded");
        self.buffer = self.buffer.replace("\r\n", "\n");
        let mut frames = Vec::new();
        while let Some(end) = self.buffer.find("\n\n") {
            frames.push(self.buffer[..end].to_string());
            self.buffer.drain(..end + 2);
        }
        Ok(frames)
    }
}

async fn wait_event(
    response: &mut Response,
    event_type: &str,
    session: Option<&str>,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut bytes = Vec::new();
        let mut frames = Frames::default();
        while let Some(chunk) = response.chunk().await? {
            bytes.extend_from_slice(&chunk);
            // Retain incomplete UTF-8 bytes across network chunks.
            let valid = match std::str::from_utf8(&bytes) {
                Ok(s) => s.len(),
                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                Err(_) => bail!("invalid event UTF-8"),
            };
            let text = std::str::from_utf8(&bytes[..valid])?;
            for frame in frames.push(text)? {
                let data = frame
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:"))
                    .map(|line| line.strip_prefix(' ').unwrap_or(line))
                    .collect::<Vec<_>>()
                    .join("\n");
                if data.is_empty() {
                    continue;
                }
                let event: Value = serde_json::from_str(&data).context("invalid SSE JSON")?;
                if event.get("type").and_then(Value::as_str) == Some(event_type)
                    && session.is_none_or(|id| data.contains(id))
                {
                    return Ok(());
                }
            }
            bytes.drain(..valid);
        }
        bail!("event stream ended before expected event")
    })
    .await
    .context("expected event timed out")?
}

fn data_id(value: &Value) -> Result<String> {
    let id = value
        .pointer("/data/id")
        .and_then(Value::as_str)
        .context("missing data.id")?;
    ensure!(
        id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "unsafe API id"
    );
    Ok(id.to_owned())
}

async fn smoke(api: &Api, directory: PathBuf) -> Result<()> {
    fs::create_dir_all(&directory)?;
    let directory = fs::canonicalize(directory)?;
    let mut events = api.events().await?;
    wait_event(&mut events, "server.connected", None).await?;
    println!("PASS event stream connected");
    let created = api
        .post(
            "/api/session",
            json!({
                "title":"YOLO Factory integration spike (no model execution)",
                "location":{"directory":directory},
                "permissions":[{"action":"factory_spike","resource":"*","effect":"ask"}]
            }),
        )
        .await?;
    let id = data_id(&created)?;
    println!("Created test session: {id}");
    // Delete only the session created by this invocation, also on test failure.
    let result = exercise(api, &id, events).await;
    let cleanup = api
        .request(Method::DELETE, &format!("/api/session/{id}"), None)
        .await;
    if cleanup.is_err() {
        eprintln!("Cleanup failed; test session remains: {id}");
    }
    result?;
    cleanup?;
    println!("PASS test session removed");
    Ok(())
}

async fn exercise(api: &Api, id: &str, mut events: Response) -> Result<()> {
    let base = format!("/api/session/{id}");
    wait_event(&mut events, "session.created", Some(id)).await?;
    let prompt = json!({"id":format!("msg_factory_{}", id),"text":"Factory protocol probe; do not execute.","resume":false});
    let first = api.post(&format!("{base}/prompt"), prompt.clone()).await?;
    let second = api.post(&format!("{base}/prompt"), prompt).await?;
    ensure!(
        data_id(&first)? == data_id(&second)?,
        "repeated prompt changed identity"
    );
    let inbox = api.get(&format!("{base}/inbox")).await?;
    ensure!(
        inbox["data"].as_array().context("inbox shape")?.len() == 1,
        "prompt duplicated"
    );
    println!("PASS queued prompt / same message ID deduplicated without LLM call");

    let form = api
        .post(
            &format!("{base}/form"),
            json!({
                "title":"Factory probe", "fields":[{"key":"choice","type":"string","required":true}]
            }),
        )
        .await?;
    let form_id = data_id(&form)?;
    let pending = api.get(&format!("{base}/form")).await?;
    ensure!(
        pending["data"].as_array().context("forms shape")?.len() == 1,
        "missing form"
    );
    let reply = format!("{base}/form/{form_id}/reply");
    api.post(&reply, json!({"answer":{"choice":"confirmed"}}))
        .await?;
    let repeated = api
        .response(
            Method::POST,
            &reply,
            Some(json!({"answer":{"choice":"different"}})),
        )
        .await?;
    ensure!(
        repeated.status() == StatusCode::CONFLICT,
        "duplicate form answer did not conflict"
    );
    println!("PASS form discovered / answered / second answer rejected");

    let permission = api
        .post(
            &format!("{base}/permission"),
            json!({
                "action":"factory_spike","resources":["synthetic-resource"]
            }),
        )
        .await?;
    ensure!(
        permission.pointer("/data/effect").and_then(Value::as_str) == Some("ask"),
        "expected permission question"
    );
    let permission_id = data_id(&permission)?;
    let pending = api.get(&format!("{base}/permission")).await?;
    ensure!(
        pending["data"]
            .as_array()
            .context("permissions shape")?
            .iter()
            .any(|v| v["id"] == permission_id),
        "permission missing"
    );
    api.post(
        &format!("{base}/permission/{permission_id}/reply"),
        json!({"decision":"reject"}),
    )
    .await?;
    println!("PASS permission discovered / explicitly rejected");

    let interrupt = api
        .request(
            Method::POST,
            &format!("{base}/interrupt?resume=false"),
            None,
        )
        .await?;
    ensure!(
        interrupt["interrupted"] == false,
        "idle session unexpectedly active"
    );
    println!("PASS idle interrupt (does not prove running tool termination)");

    drop(events);
    let missed_form = api
        .post(
            &format!("{base}/form"),
            json!({
                "title":"Created while disconnected", "fields":[{"key":"choice","type":"string"}]
            }),
        )
        .await?;
    let missed_id = data_id(&missed_form)?;
    let mut reconnected = api.events().await?;
    wait_event(&mut reconnected, "server.connected", None).await?;
    api.get(&base).await?;
    api.get(&format!("{base}/message")).await?;
    ensure!(
        api.get(&format!("{base}/form")).await?["data"]
            .as_array()
            .context("forms shape")?
            .iter()
            .any(|v| v["id"] == missed_id),
        "missed form not recovered by resync"
    );
    api.post(
        &format!("{base}/form/{missed_id}/reply"),
        json!({"answer":{"choice":"recovered"}}),
    )
    .await?;
    ensure!(
        api.get(&format!("{base}/permission")).await?["data"]
            .as_array()
            .context("permissions shape")?
            .is_empty(),
        "permission still pending"
    );
    println!("PASS disconnected form recovered via resync / history readable");
    // Remove parked input before testing the session shell, so no prompt can resume.
    api.request(
        Method::DELETE,
        &format!("{base}/inbox/{}", data_id(&first)?),
        None,
    )
    .await?;
    let shell_path = format!("{base}/shell");
    let shell = api.post(&shell_path, json!({"command":"sleep 3"}));
    let interrupt = async {
        wait_event(&mut reconnected, "session.shell.started", Some(id)).await?;
        let result = api
            .request(
                Method::POST,
                &format!("{base}/interrupt?resume=false"),
                None,
            )
            .await?;
        println!(
            "OBSERVED running shell interrupt: interrupted={}",
            result["interrupted"]
        );
        Ok::<_, anyhow::Error>(())
    };
    let (shell_result, interrupt_result) = tokio::join!(shell, interrupt);
    shell_result?;
    interrupt_result?;
    println!("PASS bounded shell call completed; process-tree termination still requires a dedicated test");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("help");
    if !matches!(mode, "inspect" | "smoke") {
        println!("Usage: factory-opencode-spike <inspect|smoke> <service-registration.json> [test-directory]\nReads local registration; credentials are never printed. smoke creates and deletes its own session; no LLM call.");
        return Ok(());
    }
    let path = args
        .get(2)
        .context("explicit service registration path required")?;
    let registration =
        serde_json::from_slice(&fs::read(path).context("cannot read registration")?)?;
    let api = Api::new(registration)?;
    let info = api.get("/api/info").await?;
    ensure!(
        info["version"] == "2.0.27",
        "spike requires verified OpenCode 2.0.27"
    );
    println!("PASS authenticated API: OpenCode 2.0.27");
    let unauth = api
        .client
        .get(format!(
            "{}/api/info",
            api.registration.url.trim_end_matches('/')
        ))
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    ensure!(
        unauth.status() == StatusCode::UNAUTHORIZED,
        "unauthenticated API not rejected"
    );
    println!("PASS unauthenticated request rejected");
    let integration = api.get("/api/integration/openai").await?;
    for method in integration["data"]["methods"]
        .as_array()
        .context("integration methods missing")?
    {
        println!(
            "OpenAI method: {} ({})",
            method["id"].as_str().unwrap_or("default"),
            method["type"].as_str().unwrap_or("unknown")
        );
    }
    if mode == "smoke" {
        let directory = args
            .get(3)
            .context("explicit disposable workspace directory required")?;
        smoke(&api, directory.into()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_handle_split_crlf_and_multiple_events() {
        let mut f = Frames::default();
        assert!(f.push("data: one\r").unwrap().is_empty());
        assert_eq!(
            f.push("\n\r\ndata: two\n\npartial").unwrap(),
            vec!["data: one", "data: two"]
        );
        assert_eq!(f.push("\n\n").unwrap(), vec!["partial"]);
    }
    #[test]
    fn refuses_remote_plaintext_and_url_credentials() {
        for url in [
            "http://example.com",
            "http://secret@localhost",
            "http://localhost/?token=secret",
        ] {
            assert!(Api::new(Registration {
                url: url.into(),
                password: "secret".into()
            })
            .is_err());
        }
    }
}
