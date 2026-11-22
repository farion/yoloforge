//! Native SSH attachment. Credentials and host-key policy stay with the user's SSH setup.
use anyhow::{ensure, Context, Result};
use factory_core::{CommandId, RunnerId};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, process::Command};
mod editor;
pub use editor::Dialog;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Profile {
    host: String,
    server: String,
    journal: String,
    #[serde(default)]
    opencode_registration: Option<String>,
}
fn config_path() -> PathBuf {
    std::env::var_os("YOLOFORGE_SSH_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
                })
                .join("yoloforge/ssh.json")
        })
}
pub fn configured(runner: RunnerId, id: CommandId) -> Result<Option<Command>> {
    configured_mode(runner, id, false)
}
pub fn readiness(runner: RunnerId, id: CommandId) -> Result<Option<Command>> {
    configured_mode(runner, id, true)
}
pub fn fetch_repository(runner: RunnerId, id: CommandId) -> Result<Option<Command>> {
    configured_operation(runner, id, true, true)
}
pub fn project_readiness(
    runner: RunnerId,
    id: CommandId,
    provider: &str,
    model: &str,
) -> Result<Option<Command>> {
    let Some(mut command) = readiness(runner, id)? else {
        return Ok(None);
    };
    let mut args = command
        .get_args()
        .map(|s| s.to_os_string())
        .collect::<Vec<_>>();
    let remote = args.pop().context("SSH remote command missing")?;
    let remote = format!(
        "{} '--inspect-model' {}",
        remote.to_string_lossy(),
        quote(&format!("{provider}/{model}"))?
    );
    // Rebuild the command because std::process::Command cannot replace an argument.
    command = Command::new("ssh");
    command
        .args(args)
        .arg(remote)
        .env_remove("FACTORY_TOKEN")
        .env_remove("FACTORY_TOKEN_FILE")
        .env_remove("OPENCODE_PASSWORD");
    Ok(Some(command))
}
fn configured_mode(runner: RunnerId, id: CommandId, readiness: bool) -> Result<Option<Command>> {
    configured_operation(runner, id, readiness, false)
}
fn configured_operation(
    runner: RunnerId,
    id: CommandId,
    readiness: bool,
    fetch: bool,
) -> Result<Option<Command>> {
    let path = config_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("SSH-Profil nicht lesbar"),
    };
    let profiles: BTreeMap<RunnerId, Profile> =
        serde_json::from_slice(&bytes).context("Ungueltiges YoloForge-SSH-Profil")?;
    profiles
        .get(&runner)
        .map(|p| command_operation(p, runner, id, readiness, fetch))
        .transpose()
}
fn quote(s: &str) -> Result<String> {
    ensure!(
        !s.is_empty() && !s.chars().any(|c| c.is_control()),
        "Leeres oder ungueltiges SSH-Profilfeld"
    );
    Ok(format!("'{}'", s.replace('\'', "'\\''")))
}
fn command(p: &Profile, runner: RunnerId, id: CommandId) -> Result<Command> {
    command_mode(p, runner, id, false)
}
fn command_mode(p: &Profile, runner: RunnerId, id: CommandId, readiness: bool) -> Result<Command> {
    command_operation(p, runner, id, readiness, false)
}
fn command_operation(
    p: &Profile,
    runner: RunnerId,
    id: CommandId,
    readiness: bool,
    fetch: bool,
) -> Result<Command> {
    let url = reqwest::Url::parse(&p.server).context("Ungueltige Factory-URL im SSH-Profil")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Factory-URL darf keine Zugangsdaten enthalten"
    );
    ensure!(
        !p.host.starts_with('-')
            && !p.host.is_empty()
            && p.host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-@:[]".contains(&b)),
        "SSH-Ziel muss ein Host oder SSH-Alias sein"
    );
    let mut args = vec![
        "factory-runner".to_string(),
        "--id".into(),
        runner.to_string(),
        "--name".into(),
        "native-attach".into(),
        "--server".into(),
        p.server.clone(),
        "--journal".into(),
        p.journal.clone(),
        if fetch {
            "--fetch-repository".into()
        } else {
            "--open-session".into()
        },
        id.to_string(),
    ];
    if let Some(path) = &p.opencode_registration {
        args.extend(["--opencode-registration".into(), path.clone()]);
    }
    if readiness && !fetch {
        args.push("--session-readiness".into());
    }
    let remote = args
        .iter()
        .map(|s| quote(s))
        .collect::<Result<Vec<_>>>()?
        .join(" ");
    let mut command = Command::new("ssh");
    if readiness {
        command.args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]);
    } else {
        command.arg("-t");
    }
    command
        .args(["--", &p.host, &remote])
        .env_remove("FACTORY_TOKEN")
        .env_remove("FACTORY_TOKEN_FILE")
        .env_remove("OPENCODE_PASSWORD");
    Ok(command)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fetch_is_noninteractive_and_pins_the_prepared_command() {
        let runner = RunnerId::new();
        let id = CommandId::new();
        let p = Profile {
            host: "user@runner".into(),
            server: "http://127.0.0.1:7432".into(),
            journal: "/private/runner journal".into(),
            opencode_registration: None,
        };
        let c = command_operation(&p, runner, id, true, true).unwrap();
        let args = c
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(args[0], "-T");
        let remote = args.last().unwrap();
        assert!(remote.contains(&format!("'--fetch-repository' '{}'", id)));
        assert!(!remote.contains("--open-session"));
        assert!(!remote.contains("--session-readiness"));
        assert!(remote.contains("'/private/runner journal'"));
    }
    #[test]
    fn ssh_uses_exact_ids_and_quotes_remote_paths_without_passwords() {
        let runner = RunnerId::new();
        let id = CommandId::new();
        let mut p = Profile {
            host: "user@runner".into(),
            server: "http://127.0.0.1:7432".into(),
            journal: "/home/user/a 'quoted' journal; echo ignored".into(),
            opencode_registration: None,
        };
        let c = command(&p, runner, id).unwrap();
        let args = c
            .get_args()
            .map(|v| v.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(&args[..3], ["-t", "--", "user@runner"]);
        assert!(args[3].contains(&id.to_string()));
        assert!(args[3].contains("'\\''quoted'\\''"));
        let status = command_mode(&p, runner, id, true).unwrap();
        let status_args = status
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            &status_args[..5],
            ["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]
        );
        assert!(status_args
            .last()
            .unwrap()
            .contains("'--session-readiness'"));
        assert!(status_args.last().unwrap().contains(&id.to_string()));
        p.host = "-oProxyCommand=anything".into();
        assert!(command(&p, runner, id).is_err());
        assert!(quote("bad\npath").is_err());
    }
}
