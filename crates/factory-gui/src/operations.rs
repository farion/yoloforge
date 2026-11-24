//! Explicit, noninteractive runner helpers. SSH quoting/host-key policy is shared with TUI.
use crate::backend::Failure;
use anyhow::{ensure, Context, Result};
use factory_client_core::ssh;
use factory_core::*;
use serde_json::{json, Value};

pub async fn execute(method: &str, path: &str, body: &Value) -> Result<Value, Failure> {
    let result = match method {
        "LOCAL_SSH_READ" => {
            let path = path.to_string();
            tokio::task::spawn_blocking(move || -> Result<Value> {
                let runner: RunnerId = path
                    .rsplit('/')
                    .next()
                    .context("Runner-ID fehlt")?
                    .parse()?;
                let profile = ssh::read_profile(runner)?;
                Ok(json!({"runner": runner, "profile": profile, "expected": profile}))
            })
            .await
            .map_err(|e| anyhow::anyhow!(e))
            .and_then(|v| v)
        }
        "LOCAL_SSH_SAVE" => {
            let body = body.clone();
            tokio::task::spawn_blocking(move || -> Result<Value> {
                let runner: RunnerId = serde_json::from_value(body["runner"].clone())?;
                let expected: Option<ssh::Profile> =
                    serde_json::from_value(body["expected"].clone())?;
                let profile: Option<ssh::Profile> =
                    serde_json::from_value(body["profile"].clone())?;
                ssh::save_profile(runner, expected, profile)?;
                Ok(json!({"saved":true}))
            })
            .await
            .map_err(|e| anyhow::anyhow!(e))
            .and_then(|v| v)
        }
        "RUNNER_HELPER" => runner_helper(body).await,
        _ => Err(anyhow::anyhow!("Unbekannte lokale Operation")),
    };
    result.map_err(|e| Failure {
        uncertain: method == "RUNNER_HELPER"
            && body["operation"] != "readiness"
            && e.is::<tokio::time::error::Elapsed>(),
        message: e.to_string(),
    })
}

async fn runner_helper(body: &Value) -> Result<Value> {
    let runner: RunnerId = serde_json::from_value(body["runner_id"].clone())?;
    let id: CommandId = serde_json::from_value(body["command_id"].clone())?;
    let operation = body["operation"].as_str().context("Operation fehlt")?;
    let get = |key: &str| body[key].as_str().unwrap_or("");
    let remote = get("transport") == "ssh";
    let command = if remote {
        match operation {
            "readiness" => ssh::project_readiness(runner, id, get("provider_id"), get("model_id"))?,
            "fetch" => ssh::fetch_repository(runner, id)?,
            "publish" => ssh::publish_review(runner, id, get("hosting_config"), get("token_file"))?,
            _ => anyhow::bail!("Nicht unterstützte Runner-Operation"),
        }.context("Kein SSH-Profil vorhanden; zuerst in der Runneransicht konfigurieren. Kein lokaler Fallback.")?
    } else {
        ensure!(ssh::read_profile(runner)?.is_none(), "Für diesen Runner existiert ein SSH-Profil. Transport ausdrücklich auf SSH stellen oder Profil entfernen.");
        ensure!(
            std::path::Path::new(get("journal")).is_absolute(),
            "Absoluten Runner-Journalpfad angeben"
        );
        let executable = std::env::current_exe()?
            .parent()
            .context("Programmverzeichnis fehlt")?
            .join("factory-runner");
        let mut command = std::process::Command::new(executable);
        command.args([
            "--id",
            &runner.to_string(),
            "--name",
            "desktop-helper",
            "--server",
            get("server"),
            "--journal",
            get("journal"),
        ]);
        match operation {
            "readiness" => {
                command.args([
                    "--open-session",
                    &id.to_string(),
                    "--session-readiness",
                    "--inspect-model",
                    &format!("{}/{}", get("provider_id"), get("model_id")),
                ]);
            }
            "fetch" => {
                command.args(["--fetch-repository", &id.to_string()]);
            }
            "publish" => {
                command.args([
                    "--publish-review",
                    &id.to_string(),
                    "--hosting-config",
                    get("hosting_config"),
                ]);
            }
            _ => anyhow::bail!("Nicht unterstützte Runner-Operation"),
        }
        if !get("opencode_registration").is_empty() {
            command.args(["--opencode-registration", get("opencode_registration")]);
        }
        if !get("token_file").is_empty() {
            command
                .env_remove("FACTORY_TOKEN")
                .env("FACTORY_TOKEN_FILE", get("token_file"));
        }
        command
    };
    let mut command = tokio::process::Command::from(command);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(180), command.output())
        .await
        .context("Runner-Helfer ohne Bestätigung abgebrochen; Ausgang vor Wiederholung prüfen")??;
    // Never hide unsuccessful status; preserve complete helper output in the private result.
    let result = json!({"success": output.status.success(), "exit_code": output.status.code(), "stdout": String::from_utf8_lossy(&output.stdout), "stderr": String::from_utf8_lossy(&output.stderr)});
    Ok(result)
}
