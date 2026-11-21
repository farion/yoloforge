//! Freeze a dedicated runtime, never infer process quiescence from an interrupt.
use crate::{
    journal::{Journal, Record},
    Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_opencode::OpenCodeClient;
use factory_protocol::{
    client::{ClientError, FactoryClient},
    ErrorCode, ReportControl,
};
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
    #[serde(default)]
    daemon_id: Option<String>,
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
pub(crate) async fn docker(config: &Config, args: &[&str]) -> Result<Vec<u8>> {
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
    if v["State"]["Running"] == true {
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
    }
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
    daemon(c).await?;
    let values: Vec<Value> = serde_json::from_slice(
        &docker(c, &["inspect", "--type", "container", &c.container_id]).await?,
    )?;
    ensure!(values.len() == 1, "Ambiguous inspection");
    let v = values.into_iter().next().unwrap();
    validate(&v, c, runner, endpoint, workspace)?;
    validate_volumes(c, runner, &v).await?;
    Ok(v)
}
pub(crate) async fn daemon(c: &Config) -> Result<String> {
    let info: Value =
        serde_json::from_slice(&docker(c, &["info", "--format", "{{json .}} "]).await?)?;
    ensure!(
        info["OSType"] == "linux" && info["CgroupVersion"] == "2",
        "Linux cgroup v2 required"
    );
    let id = info["ID"]
        .as_str()
        .context("Docker daemon identity missing")?;
    ensure!(!id.is_empty(), "Docker daemon identity missing");
    Ok(id.into())
}
async fn validate_volumes(c: &Config, runner: RunnerId, v: &Value) -> Result<()> {
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
    Ok(())
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
    let daemon_id = Some(daemon(&config).await?);
    let v = inspect(&config, command.runner_id, endpoint, workspace).await?;
    ensure!(
        v["State"]["Running"] == true && v["State"]["Restarting"] == false,
        "Runtime not running"
    );
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
        daemon_id,
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
    if let Some(id) = &saved.daemon_id {
        ensure!(&daemon(&config).await? == id, "Docker daemon changed");
    }
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
    ensure!(
        v["State"]["Running"] == true && v["State"]["Restarting"] == false,
        "Runtime not running; recovery required"
    );
    Ok(Some(v))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemovalAttempt {
    runtime: RuntimeRef,
    proof: Option<RemovalProof>,
}
async fn absent(c: &Config) -> Result<bool> {
    let filter = format!("id={}", c.container_id);
    let result = docker(c, &["ps", "-aq", "--no-trunc", "--filter", &filter]).await?;
    let result = std::str::from_utf8(&result)?.trim();
    ensure!(
        result.is_empty() || result == c.container_id,
        "Ambiguous runtime lookup"
    );
    Ok(result.is_empty())
}
pub async fn verify_removed(args: &Args, stream: StreamId, proof: &RemovalProof) -> Result<()> {
    let mut c = config(args, stream)?;
    c.container_id = proof.runtime.container_id.clone();
    ensure!(
        daemon(&c).await? == proof.daemon_id,
        "Checkpoint daemon differs from removal proof"
    );
    ensure!(absent(&c).await?, "Checkpoint runtime is still present");
    Ok(())
}
pub async fn remove(
    journal: &Journal,
    record: &mut Record,
    args: &Args,
    request: &RuntimeTermination,
) -> Result<RemovalProof> {
    ensure!(
        request.command_id == record.command.id
            && request.stream_id == record.command.stream_id
            && request.runner_id == record.command.runner_id
            && request.ownership_epoch == record.command.ownership_epoch,
        "Foreign termination request"
    );
    let local = record.agent.as_ref().and_then(|a| a.runtime.as_ref());
    let recovered = request
        .binding
        .as_ref()
        .map(|b| -> Result<Identity> {
            ensure!(
                b.command_id == record.command.id
                    && b.stream_id == record.command.stream_id
                    && b.runner_id == record.command.runner_id
                    && b.ownership_epoch == record.command.ownership_epoch
                    && b.runtime == request.runtime,
                "Foreign durable runtime binding"
            );
            Ok(Identity {
                config: config(args, b.stream_id)?,
                daemon_id: Some(b.daemon_id.clone()),
                started_at: b.runtime.started_at.clone(),
                endpoint: b.endpoint.clone(),
                workspace: b.workspace.clone(),
            })
        })
        .transpose()?;
    let saved = local
        .or(recovered.as_ref())
        .context("Runtime journal and central binding missing")?
        .clone();
    ensure!(
        saved.config == config(args, record.command.stream_id)?,
        "Runtime configuration changed"
    );
    ensure!(
        request.runtime.container_id == saved.config.container_id
            && request.runtime.started_at == saved.started_at,
        "Runtime binding differs"
    );
    let daemon_id = saved
        .daemon_id
        .as_ref()
        .context("Legacy runtime lacks daemon pin; no automatic adoption")?;
    ensure!(
        &daemon(&saved.config).await? == daemon_id,
        "Docker daemon changed"
    );
    if let Some(attempt) = record.removals.get(&request.request_id) {
        ensure!(
            attempt.runtime == request.runtime,
            "Removal request changed"
        );
    }
    if !absent(&saved.config).await? {
        let v = inspect(
            &saved.config,
            record.command.runner_id,
            &saved.endpoint,
            &saved.workspace,
        )
        .await?;
        ensure!(
            v["State"]["StartedAt"] == saved.started_at && v["State"]["Restarting"] == false,
            "Runtime incarnation changed"
        );
        ensure!(
            !record.removals.contains_key(&request.request_id),
            "Removal outcome uncertain; read-only reconciliation only"
        );
        record.removals.insert(
            request.request_id,
            RemovalAttempt {
                runtime: request.runtime.clone(),
                proof: None,
            },
        );
        tokio::task::block_in_place(|| journal.save(record))?;
        // No --volumes: retain native OpenCode data and all bind-mounted work.
        let _ = docker(
            &saved.config,
            &["rm", "--force", &saved.config.container_id],
        )
        .await;
    }
    ensure!(
        &daemon(&saved.config).await? == daemon_id,
        "Docker daemon changed during removal"
    );
    ensure!(
        absent(&saved.config).await?,
        "Runtime removal not confirmed"
    );
    let proof = record
        .removals
        .get(&request.request_id)
        .and_then(|a| a.proof.clone())
        .unwrap_or(RemovalProof {
            runtime: request.runtime.clone(),
            daemon_id: daemon_id.clone(),
            observed_unix_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64,
        });
    record.removals.insert(
        request.request_id,
        RemovalAttempt {
            runtime: request.runtime.clone(),
            proof: Some(proof.clone()),
        },
    );
    tokio::task::block_in_place(|| journal.save(record))?;
    Ok(proof)
}
pub async fn publish_binding(
    factory: &FactoryClient,
    journal: &Journal,
    record: &mut Record,
    registration: RegistrationId,
) -> Result<()> {
    if record.runtime_published {
        return Ok(());
    }
    let Some(saved) = record.agent.as_ref().and_then(|a| a.runtime.as_ref()) else {
        return Ok(());
    };
    // Legacy executions can still be observed, but cannot acquire an invented daemon pin.
    let Some(daemon_id) = saved.daemon_id.clone() else {
        return Ok(());
    };
    let binding = RuntimeBinding {
        command_id: record.command.id,
        stream_id: record.command.stream_id,
        runner_id: record.command.runner_id,
        ownership_epoch: record.command.ownership_epoch,
        runtime: RuntimeRef {
            container_id: saved.config.container_id.clone(),
            started_at: saved.started_at.clone(),
        },
        daemon_id,
        endpoint: saved.endpoint.clone(),
        workspace: saved.workspace.clone(),
    };
    ensure!(
        factory
            .report_runtime_binding(
                record.command.runner_id,
                registration,
                record.command.id,
                &binding
            )
            .await?
            == binding,
        "Factory runtime binding differs"
    );
    record.runtime_published = true;
    tokio::task::block_in_place(|| journal.save(record))
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
    let reported = factory
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
        .await;
    if matches!(
        &reported,
        Err(ClientError::Http {
            code: Some(ErrorCode::WorkConflict),
            ..
        })
    ) {
        let latest = factory
            .runner_control(
                record.command.runner_id,
                registration,
                record.command.id,
                record.command.ownership_epoch,
            )
            .await?;
        if latest.is_some_and(|c| {
            c.command_id == Some(record.command.id) && c.request_id != control.request_id
        }) {
            // Recovery superseded the observed request while Docker was being
            // inspected. Keep its receipt; process the new request next tick.
            return Ok(true);
        }
        if factory
            .runner_termination(
                record.command.runner_id,
                registration,
                record.command.id,
                record.command.ownership_epoch,
            )
            .await?
            .is_some()
        {
            return Ok(true);
        }
    }
    reported?;
    Ok(true)
}
