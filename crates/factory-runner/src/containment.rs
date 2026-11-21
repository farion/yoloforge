//! Freeze a dedicated runtime, never infer process quiescence from an interrupt.
use crate::{
    journal::{Journal, Record},
    Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_opencode::OpenCodeClient;
use factory_protocol::{client::FactoryClient, ReportControl};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::process::Command;

#[cfg(all(test, unix))]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub stream_id: StreamId,
    pub container_id: String,
    pub docker_socket: PathBuf,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    config: Config,
    started_at: String,
    endpoint: String,
    workspace: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attempt {
    action: ControlAction,
}

fn config(args: &Args, stream: StreamId) -> Result<Config> {
    let path = args
        .containment
        .as_ref()
        .context("Containment configuration missing")?;
    let mut config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        config.stream_id == stream,
        "Dedicated runtime belongs to another stream"
    );
    ensure!(
        config.container_id.len() == 64
            && config.container_id.bytes().all(|b| b.is_ascii_hexdigit()),
        "Full immutable container ID required"
    );
    config.docker_socket = std::fs::canonicalize(&config.docker_socket)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        ensure!(
            std::fs::metadata(&config.docker_socket)?
                .file_type()
                .is_socket(),
            "Docker endpoint must be a local Unix socket"
        );
    }
    Ok(config)
}
async fn docker(config: &Config, args: &[&str]) -> Result<Vec<u8>> {
    let mut command = Command::new("docker");
    command
        .arg("--host")
        .arg(format!("unix://{}", config.docker_socket.display()))
        .args(args)
        .env_remove("DOCKER_HOST")
        .env_remove("DOCKER_CONTEXT")
        .env_remove("DOCKER_TLS_VERIFY")
        .env_remove("DOCKER_CERT_PATH")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .context("Docker outcome uncertain")??;
    #[cfg(test)]
    if !output.status.success() {
        eprintln!(
            "Docker test diagnostic: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    ensure!(output.status.success(), "Docker operation failed");
    ensure!(
        output.stdout.len() <= 1024 * 1024,
        "Docker response too large"
    );
    Ok(output.stdout)
}
fn empty(v: &Value) -> bool {
    v.is_null()
        || v.as_array().is_some_and(Vec::is_empty)
        || v.as_object().is_some_and(|v| v.is_empty())
}
fn contains(v: &Value, s: &str) -> bool {
    v.as_array().is_some_and(|a| a.iter().any(|v| v == s))
}
fn validate(
    v: &Value,
    c: &Config,
    runner: RunnerId,
    endpoint: &str,
    workspace: &str,
) -> Result<()> {
    ensure!(
        v["Id"] == c.container_id && v["Platform"] == "linux",
        "Container identity/platform changed"
    );
    ensure!(
        v["Config"]["Labels"]["io.yoloforge.stream"] == c.stream_id.to_string()
            && v["Config"]["Labels"]["io.yoloforge.runner"] == runner.to_string(),
        "Runtime labels differ"
    );
    let h = &v["HostConfig"];
    ensure!(
        h["Privileged"] == false
            && h["ReadonlyRootfs"] == true
            && h["CgroupnsMode"] == "private"
            && (h["PidMode"] == "" || h["PidMode"] == "private")
            && h["IpcMode"] == "private",
        "Runtime namespace/rootfs isolation invalid"
    );
    ensure!(
        h["NetworkMode"] != "host"
            && !h["NetworkMode"]
                .as_str()
                .unwrap_or_default()
                .starts_with("container:"),
        "Private network namespace required"
    );
    ensure!(
        empty(&h["CapAdd"])
            && contains(&h["CapDrop"], "ALL")
            && empty(&h["Devices"])
            && empty(&h["DeviceRequests"])
            && empty(&h["VolumesFrom"]),
        "Runtime privileges invalid"
    );
    let security = h["SecurityOpt"]
        .as_array()
        .context("Runtime security options missing")?;
    ensure!(
        !security.is_empty()
            && security
                .iter()
                .all(|v| v == "no-new-privileges" || v == "no-new-privileges:true"),
        "Only no-new-privileges with default security profiles is supported"
    );
    ensure!(
        h["RestartPolicy"]["Name"] == "no" || h["RestartPolicy"]["Name"] == "",
        "Automatic restart forbidden"
    );
    let user = v["Config"]["User"]
        .as_str()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    ensure!(
        user.parse::<u32>().is_ok_and(|uid| uid > 0),
        "Explicit non-root numeric UID required"
    );
    let port = endpoint
        .trim_end_matches('/')
        .strip_prefix("http://127.0.0.1:")
        .context("Loopback HTTP endpoint with explicit port required")?
        .parse::<u16>()?;
    let ports = v["NetworkSettings"]["Ports"]["4096/tcp"]
        .as_array()
        .context("Runtime port missing")?;
    ensure!(
        ports.len() == 1
            && ports[0]["HostIp"] == "127.0.0.1"
            && ports[0]["HostPort"]
                .as_str()
                .and_then(|v| v.parse::<u16>().ok())
                == Some(port),
        "Endpoint is not the dedicated runtime port"
    );
    if let Some(tmpfs) = h["Tmpfs"].as_object() {
        ensure!(
            tmpfs.keys().all(|p| p == "/tmp" || p == "/run"),
            "Unexpected tmpfs path"
        );
    }
    let mounts = v["Mounts"].as_array().context("Runtime mounts missing")?;
    let mut worktree = false;
    for m in mounts {
        match m["Type"].as_str() {
            Some("bind") => {
                let source = m["Source"].as_str().context("Bind source missing")?;
                let destination = m["Destination"]
                    .as_str()
                    .context("Bind destination missing")?;
                let work = source == workspace && destination == workspace && m["RW"] == true;
                let git = source == destination
                    && source.ends_with("/.git")
                    && m["RW"] == false
                    && Path::new(source).is_dir();
                ensure!(
                    work || git,
                    "Only exact worktree and read-only Git metadata binds supported"
                );
                worktree |= work;
            }
            Some("volume") => {}
            Some("tmpfs") => ensure!(
                matches!(m["Destination"].as_str(), Some("/tmp" | "/run")),
                "Unexpected tmpfs"
            ),
            _ => anyhow::bail!("Unsupported mount"),
        }
    }
    ensure!(worktree, "Exact absolute worktree bind required");
    Ok(())
}
async fn inspect(c: &Config, runner: RunnerId, endpoint: &str, workspace: &str) -> Result<Value> {
    let info: Value =
        serde_json::from_slice(&docker(c, &["info", "--format", "{{json .}} "]).await?)?;
    ensure!(
        info["OSType"] == "linux" && info["CgroupVersion"] == "2",
        "Linux cgroup v2 required"
    );
    let values: Vec<Value> = serde_json::from_slice(
        &docker(c, &["inspect", "--type", "container", &c.container_id]).await?,
    )?;
    ensure!(values.len() == 1, "Ambiguous inspection");
    let v = values.into_iter().next().unwrap();
    validate(&v, c, runner, endpoint, workspace)?;
    for m in v["Mounts"].as_array().unwrap() {
        if m["Type"] == "volume" {
            let name = m["Name"].as_str().context("Volume name missing")?;
            ensure!(!name.starts_with('-'), "Invalid volume name");
            let volumes: Vec<Value> =
                serde_json::from_slice(&docker(c, &["volume", "inspect", name]).await?)?;
            ensure!(
                volumes.len() == 1
                    && volumes[0]["Driver"] == "local"
                    && empty(&volumes[0]["Options"])
                    && volumes[0]["Labels"]["io.yoloforge.stream"] == c.stream_id.to_string()
                    && volumes[0]["Labels"]["io.yoloforge.runner"] == runner.to_string(),
                "Volume is not dedicated local storage"
            );
            let filter = format!("volume={name}");
            let users = docker(c, &["ps", "-aq", "--no-trunc", "--filter", &filter]).await?;
            ensure!(
                std::str::from_utf8(&users)?
                    .lines()
                    .all(|id| id == c.container_id),
                "Volume shared with another container"
            );
        }
    }
    ensure!(
        v["State"]["Running"] == true && v["State"]["Restarting"] == false,
        "Runtime not running; recovery required"
    );
    Ok(v)
}
pub async fn bind(
    args: &Args,
    command: &RunnerCommand,
    endpoint: &str,
    workspace: &str,
) -> Result<Option<Identity>> {
    if args.containment.is_none() {
        return Ok(None);
    }
    let config = config(args, command.stream_id)?;
    let v = inspect(&config, command.runner_id, endpoint, workspace).await?;
    ensure!(
        v["State"]["Paused"] == false,
        "Runtime frozen before binding"
    );
    let started_at = v["State"]["StartedAt"]
        .as_str()
        .context("Runtime incarnation missing")?
        .to_string();
    ensure!(!started_at.is_empty(), "Runtime incarnation missing");
    Ok(Some(Identity {
        config,
        started_at,
        endpoint: endpoint.into(),
        workspace: workspace.into(),
    }))
}
async fn current(record: &Record, args: &Args) -> Result<Option<Value>> {
    let saved = record.agent.as_ref().and_then(|a| a.runtime.as_ref());
    if args.containment.is_none() && saved.is_none() {
        return Ok(None);
    }
    let config = config(args, record.command.stream_id)?;
    let saved = saved.context("Cannot adopt an existing unmanaged execution")?;
    ensure!(saved.config == config, "Runtime configuration changed");
    let api = OpenCodeClient::from_registration(
        args.opencode_registration
            .as_ref()
            .context("Registration missing")?,
    )?;
    ensure!(api.endpoint() == saved.endpoint, "Runtime endpoint changed");
    let v = inspect(
        &config,
        record.command.runner_id,
        &saved.endpoint,
        &saved.workspace,
    )
    .await?;
    ensure!(
        v["State"]["StartedAt"] == saved.started_at,
        "Runtime incarnation changed"
    );
    Ok(Some(v))
}
pub async fn ready(record: &Record, args: &Args) -> Result<()> {
    if record.command.kind != CommandKind::RunAgent {
        return Ok(());
    }
    if record.agent.is_none() {
        if args.containment.is_some() {
            config(args, record.command.stream_id)?;
        }
        return Ok(());
    }
    if let Some(v) = current(record, args).await? {
        ensure!(v["State"]["Paused"] == false, "Runtime unexpectedly frozen");
    }
    Ok(())
}
async fn transition(
    journal: &Journal,
    record: &mut Record,
    args: &Args,
    control: &StreamControl,
) -> Result<ContainmentProof> {
    ensure!(
        control.stream_id == record.command.stream_id
            && control.command_id == Some(record.command.id),
        "Control belongs to another execution"
    );
    let mut v = current(record, args)
        .await?
        .context("Runtime binding missing")?;
    let identity = record
        .agent
        .as_ref()
        .and_then(|a| a.runtime.as_ref())
        .unwrap()
        .clone();
    if let Some(proof) = &control.proof {
        ensure!(
            proof.container_id == identity.config.container_id
                && proof.started_at == identity.started_at,
            "Server runtime identity differs"
        );
    }
    let frozen = control.action == ControlAction::Pause;
    if let Some(attempt) = record.container_actions.get(&control.request_id) {
        ensure!(attempt.action == control.action, "Control action changed");
    }
    let observed = v["State"]["Paused"]
        .as_bool()
        .context("Freezer state missing")?;
    if observed != frozen {
        ensure!(
            !record.container_actions.contains_key(&control.request_id),
            "Freezer outcome uncertain; read-only reconciliation required"
        );
        record.container_actions.insert(
            control.request_id,
            Attempt {
                action: control.action,
            },
        );
        tokio::task::block_in_place(|| journal.save(record))?;
        let _ = docker(
            &identity.config,
            &[
                if frozen { "pause" } else { "unpause" },
                &identity.config.container_id,
            ],
        )
        .await;
        v = current(record, args)
            .await?
            .context("Runtime disappeared")?;
    }
    ensure!(v["State"]["Paused"] == frozen, "Freezer state unconfirmed");
    // Even an already-satisfied control must never issue a later write on state drift.
    if let std::collections::btree_map::Entry::Vacant(entry) =
        record.container_actions.entry(control.request_id)
    {
        entry.insert(Attempt {
            action: control.action,
        });
        tokio::task::block_in_place(|| journal.save(record))?;
    }
    Ok(ContainmentProof {
        container_id: identity.config.container_id.clone(),
        started_at: identity.started_at.clone(),
        frozen,
        observed_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64,
    })
}
/// Handle controls before any OpenCode access (the API itself freezes with the tools).
pub async fn control(
    factory: &FactoryClient,
    journal: &Journal,
    record: &mut Record,
    args: &Args,
    registration: RegistrationId,
) -> Result<bool> {
    if record.command.kind != CommandKind::RunAgent
        || record
            .agent
            .as_ref()
            .is_none_or(|a| !a.prompt_attempted || a.runtime.is_none())
    {
        return Ok(false);
    }
    let Some(control) = factory
        .runner_control(
            record.command.runner_id,
            registration,
            record.command.id,
            record.command.ownership_epoch,
        )
        .await?
        .filter(|c| c.state != PauseState::Released && c.command_id == Some(record.command.id))
    else {
        return Ok(false);
    };
    let (state, summary, proof) = match transition(journal, record, args, &control).await {
        Ok(proof) if proof.frozen => (
            PauseState::Frozen,
            "Dedicated container freezer confirmed; ownership retained",
            Some(proof),
        ),
        Ok(proof) => (
            PauseState::Released,
            "Dedicated container thaw confirmed; same execution continues",
            Some(proof),
        ),
        Err(_) => (
            PauseState::InterruptUnverified,
            "Container identity/freezer state unverified; no replacement writer or automatic retry",
            None,
        ),
    };
    factory
        .report_control(
            record.command.runner_id,
            registration,
            record.command.id,
            &ReportControl {
                proof,
                control_request: control.request_id,
                ownership_epoch: record.command.ownership_epoch,
                state,
                summary: summary.into(),
            },
        )
        .await?;
    Ok(true)
}
