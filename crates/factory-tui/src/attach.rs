//! Native terminal takeover: the original OpenCode UI, attached to the recorded session.
use anyhow::{Context, Result};
use factory_core::*;
use factory_protocol::Snapshot;
use std::path::PathBuf;

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
    use super::*;
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
