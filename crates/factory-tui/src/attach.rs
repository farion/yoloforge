//! Native terminal takeover: the original OpenCode UI, attached to the recorded session.
use anyhow::{Context, Result};
use factory_core::*;
use factory_protocol::Snapshot;
use std::path::PathBuf;

pub fn project_connection(
    snapshot: &Snapshot,
    id: ProjectId,
) -> Result<(factory_opencode::OpenCodeClient, PathBuf)> {
    let prepared = snapshot
        .commands
        .iter()
        .rev()
        .find(|c| {
            c.kind == CommandKind::PrepareWorkspace
                && c.state == CommandState::Succeeded
                && snapshot.streams.iter().any(|s| {
                    s.id == c.stream_id
                        && s.project_id == id
                        && s.ownership_epoch == c.ownership_epoch
                })
        })
        .context("Projektarbeitsordner wird noch vorbereitet; Diagnose zeigt den Fortschritt")?;
    let Some(CommandOutput::Workspace { workspace }) = &prepared.result else {
        anyhow::bail!("Bestaetigter Projektordner fehlt");
    };
    anyhow::ensure!(crate::ssh::configured(prepared.runner_id,prepared.id)?.is_none(),"Projekt liegt auf SSH-Runner; Anmeldung/Bereitschaft dort ausfuehren. SSH-Sessions lassen sich ueber Enter oeffnen");
    let registration = prepared
        .workspace_spec
        .as_ref()
        .and_then(|s| s.setup.as_ref())
        .map(|s| s.service_registration.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
                })
                .join("opencode/service.json")
        });
    let directory = std::fs::canonicalize(&workspace.path)
        .context("Projektarbeitsordner ist auf diesem TUI-Rechner nicht erreichbar")?;
    Ok((
        factory_opencode::OpenCodeClient::from_registration(&registration)
            .context("Lokale OpenCode-Verbindung fehlt")?,
        directory,
    ))
}

pub async fn project_terminal(snapshot: &Snapshot, id: ProjectId) -> Result<std::process::Command> {
    if let Some(command) = remote_execution(snapshot, id) {
        if let Some(remote) = crate::ssh::configured(command.runner_id, command.id)? {
            return Ok(remote);
        }
    }
    let (client, path) = project_connection(snapshot, id)?;
    client.project_terminal_command(&path).await
}

fn remote_execution(snapshot: &Snapshot, id: ProjectId) -> Option<&RunnerCommand> {
    let belongs = |c: &&RunnerCommand| {
        matches!(
            c.kind,
            CommandKind::RunAgent | CommandKind::PrepareWorkspace
        ) && snapshot.streams.iter().any(|s| {
            s.id == c.stream_id
                && s.project_id == id
                && s.ownership_epoch == c.ownership_epoch
                && s.runner_id == c.runner_id
        })
    };
    snapshot
        .commands
        .iter()
        .rev()
        .filter(belongs)
        .find(|c| c.kind == CommandKind::RunAgent)
        .or_else(|| {
            snapshot.commands.iter().rev().filter(belongs).find(|c| {
                c.kind == CommandKind::PrepareWorkspace && c.state == CommandState::Succeeded
            })
        })
}

async fn remote_report(command: std::process::Command) -> Result<String> {
    remote_report_timeout(command, 30).await
}
pub(crate) async fn remote_report_timeout(
    command: std::process::Command,
    seconds: u64,
) -> Result<String> {
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::from(command);
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context(
            "Runner-Helfer konnte nicht gestartet werden (lokales Programm bzw. SSH pruefen)",
        )?;
    let output = child.stdout.take().context("Runner-Helferausgabe fehlt")?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(seconds), async {
        let mut bytes = Vec::new();
        output.take(65537).read_to_end(&mut bytes).await?;
        anyhow::ensure!(bytes.len() <= 65536, "Runner-Helferausgabe ist zu gross");
        let status = child.wait().await?;
        anyhow::ensure!(status.success(), "Runner-Helfer fehlgeschlagen: Journal, Konfiguration, Zugangsdaten und bei SSH Hostschluessel/Anmeldung pruefen");
        Ok::<_,anyhow::Error>(String::from_utf8_lossy(&bytes).chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect::<String>())
    }).await.context("Runner-Helfer: Zeitlimit ueberschritten; Ergebnis vor Neuversuch abgleichen")?;
    if result.is_err() {
        let _ = child.kill().await;
    }
    result
}

pub async fn fetch_repository(snapshot: &Snapshot, id: ProjectId) -> Result<String> {
    snapshot
        .projects
        .iter()
        .find(|p| p.id == id)
        .context("Projekt fehlt")?;
    let prepared = snapshot
        .commands
        .iter()
        .rev()
        .find(|c| {
            c.kind == CommandKind::PrepareWorkspace
                && c.state == CommandState::Succeeded
                && snapshot.streams.iter().any(|s| {
                    s.id == c.stream_id
                        && s.project_id == id
                        && s.runner_id == c.runner_id
                        && s.ownership_epoch == c.ownership_epoch
                })
        })
        .context("Zuerst einen bestaetigten Projektarbeitsbereich vorbereiten")?;
    if let Some(command) = crate::ssh::fetch_repository(prepared.runner_id, prepared.id)? {
        return remote_report_timeout(command, 120).await;
    }
    let Some(CommandOutput::Workspace { workspace }) = &prepared.result else {
        anyhow::bail!("Bestaetigter Arbeitsbereich fehlt");
    };
    let path = PathBuf::from(&workspace.path);
    let branch = workspace.branch.clone();
    let path = tokio::task::spawn_blocking(move || {
        crate::commands::repository::fetch_workspace(&path, &branch)
    })
    .await??;
    let path = path
        .to_string_lossy()
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>();
    Ok(format!("Origin-Referenzen aktualisiert: {path}\nArbeitsdateien, Index und lokale Branches bleiben erhalten."))
}

pub async fn readiness(snapshot: &Snapshot, id: ProjectId) -> Result<String> {
    let project = snapshot
        .projects
        .iter()
        .find(|p| p.id == id)
        .context("Projekt fehlt")?;
    if let Some(command) = remote_execution(snapshot, id) {
        if let Some(remote) = crate::ssh::project_readiness(
            command.runner_id,
            command.id,
            &project.profile.provider_id,
            &project.profile.model_id,
        )? {
            let report = remote_report(remote).await?;
            let scope = if command.kind == CommandKind::PrepareWorkspace {
                "Vorbereiteter Projektordner; noch keine Agent-Session"
            } else {
                "Aufgezeichnete Ausfuehrung, aktuelles Projektmodell"
            };
            return Ok(format!(
                "Aktuelles Projektmodell: {}/{}\nSSH-Runner {} ({})\n{}",
                project.profile.provider_id,
                project.profile.model_id,
                command.runner_id,
                scope,
                report
            ));
        }
    }
    let (client, path) = project_connection(snapshot, id)?;
    client
        .readiness(
            path.to_str().context("Projektpfad ist nicht UTF-8")?,
            &project.profile.provider_id,
            &project.profile.model_id,
        )
        .await
}

pub async fn prepare(
    snapshot: &Snapshot,
    stream: StreamId,
    selected: Option<CommandId>,
) -> Result<std::process::Command> {
    let command = snapshot
        .commands
        .iter()
        .rev()
        .find(|c| {
            c.stream_id == stream
                && c.kind == CommandKind::RunAgent
                && selected.is_none_or(|id| id == c.id)
        })
        .context("OpenCode-Session wird noch vorbereitet")?;
    if let Some(remote) = crate::ssh::configured(command.runner_id, command.id)? {
        return Ok(remote);
    }
    let workspace = snapshot
        .commands
        .iter()
        .rev()
        .filter(|c| c.stream_id == stream && c.ownership_epoch == command.ownership_epoch)
        .find_map(|c| match &c.result {
            Some(CommandOutput::Workspace { workspace }) => Some(workspace),
            _ => None,
        })
        .context("Bestaetigter Arbeitsordner fehlt")?;
    let registration = command
        .workspace_spec
        .as_ref()
        .and_then(|s| s.setup.as_ref())
        .map(|s| s.service_registration.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
                })
                .join("opencode/service.json")
        });
    let client=factory_opencode::OpenCodeClient::from_registration(&registration)
        .context("OpenCode-Verbindung auf diesem Rechner fehlt; fuer einen entfernten Runner ist eine native Verbindung noch nicht eingerichtet")?;
    client
        .terminal_command(&command.session_id(), std::path::Path::new(&workspace.path))
        .await
}

pub fn run(mut command: std::process::Command) -> Result<String> {
    crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste)?;
    ratatui::restore();
    let result=command.status().context("OpenCode-Terminal konnte nicht gestartet werden; opencode muss auf diesem Rechner installiert sein");
    // The caller recreates its Ratatui terminal on success and failure alike.
    match result? {
        status if status.success() => Ok("Zurueck von OpenCode".into()),
        status => Ok(format!("OpenCode beendet ({status})")),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[tokio::test]
    async fn remote_readiness_bounds_and_sanitizes_output_and_rejects_failures() {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "printf 'Ready\\n\\033[31m' "]);
        let report = super::remote_report(command).await.unwrap();
        assert!(report.starts_with("Ready\n"));
        assert!(!report.contains('\u{1b}'));
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "exit 1"]);
        assert!(super::remote_report(command).await.is_err());
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "head -c 65537 /dev/zero"]);
        assert!(super::remote_report(command).await.is_err());
    }
    use super::*;
    #[tokio::test]
    async fn readiness_reports_expired_auth_and_mcp_without_exposing_credentials() {
        use serde_json::json;
        use std::io::{Read, Write};
        let dir = tempfile::tempdir().unwrap();
        let location = dir.path().to_str().unwrap().to_string();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let expected = location.clone();
        let server = std::thread::spawn(move || {
            for (path, body) in [
                ("/api/info", json!({"version":"2.0.27"})),
                (
                    "/api/integration",
                    json!({"location":{"directory":expected},"data":[{"name":"Provider","connections":[{"status":{"status":"needs_auth","message":"secret-message","url":"secret-url"}}]}]}),
                ),
                (
                    "/api/mcp",
                    json!({"location":{"directory":expected},"data":[{"name":"Docs","status":{"status":"needs_auth","error":"secret-error"}}]}),
                ),
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut b = [0u8; 1024];
                    let n = socket.read(&mut b).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&b[..n]);
                    assert!(request.len() < 8192);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(&format!("GET {path}")));
                if path != "/api/info" {
                    assert!(request.contains("location%5Bdirectory%5D="));
                }
                let body = body.to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let registration = dir.path().join("service.json");
        std::fs::write(
            &registration,
            json!({"url":format!("http://127.0.0.1:{port}"),"password":"test-secret"}).to_string(),
        )
        .unwrap();
        let client = factory_opencode::OpenCodeClient::from_registration(&registration).unwrap();
        let text = client.readiness(&location, "", "").await.unwrap();
        server.join().unwrap();
        assert!(text.contains("Projektmodell fehlt"));
        assert!(text.contains("Anmeldung abgelaufen"));
        assert!(text.contains("MCP Docs: needs_auth"));
        assert!(!text.contains("secret"));
    }
    async fn checked_command(missing: bool, wrong_location: bool) -> Result<std::process::Command> {
        use std::io::{Read, Write};
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let location = if wrong_location {
            other.path()
        } else {
            dir.path()
        }
        .to_string_lossy()
        .into_owned();
        let server = std::thread::spawn(move || {
            for (status, body) in [
                (200, serde_json::json!({"version":"2.0.27"})),
                (
                    if missing { 404 } else { 200 },
                    serde_json::json!({"data":{"id":"ses_existing","location":{"directory":location}}}),
                ),
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut bytes = [0u8; 1024];
                    let count = socket.read(&mut bytes).unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&bytes[..count]);
                    assert!(request.len() <= 8192);
                }
                let body = body.to_string();
                write!(socket,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let reg = dir.path().join("service.json");
        std::fs::write(&reg,serde_json::json!({"url":format!("http://127.0.0.1:{port}"),"password":"private-test-password"}).to_string()).unwrap();
        let client = factory_opencode::OpenCodeClient::from_registration(&reg).unwrap();
        let result = client.terminal_command("ses_existing", dir.path()).await;
        server.join().unwrap();
        result
    }
    #[tokio::test]
    async fn original_cli_receives_exact_session_but_no_password_in_arguments() {
        let command = checked_command(false, false).await.unwrap();
        let args = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.windows(2).any(|a| a == ["--session", "ses_existing"]));
        assert!(!args.iter().any(|a| a.contains("private-test-password")));
        assert!(!args.iter().any(|a| a == "--prompt" || a == "--continue"));
        assert!(command
            .get_envs()
            .any(|(k, v)| k == "OPENCODE_PASSWORD"
                && v.is_some_and(|v| v == "private-test-password")));
    }
    #[tokio::test]
    async fn refuses_missing_session_or_wrong_workspace_before_launch() {
        assert!(checked_command(true, false).await.is_err());
        assert!(checked_command(false, true).await.is_err());
    }
}
