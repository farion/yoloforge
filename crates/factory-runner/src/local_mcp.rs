use anyhow::{ensure, Result};
use factory_core::{valid_repo_skill_id, LocalMcp, RegistrationId, RemoteMcpRef, RunnerCommand};
use factory_opencode::OpenCodeClient;
use factory_protocol::client::FactoryClient;
use factory_protocol::mcp::{LocalMcpProbeStatus as ProbeStatus, LocalMcpProbeView};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

pub fn name(mcp: &LocalMcp) -> Result<String> {
    ensure!(valid_repo_skill_id(&mcp.id), "Ungültige lokale MCP-ID");
    let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&mcp.command)?));
    Ok(format!("yf-{}-{}", mcp.id, &digest[..12]))
}

pub fn remote_name(mcp: &RemoteMcpRef) -> Result<String> {
    ensure!(valid_repo_skill_id(&mcp.id), "Ungültige Remote-MCP-ID");
    Ok(format!("yfr-{}-{}", mcp.id, mcp.revision.0))
}

pub async fn test_next(factory: &FactoryClient, args: &crate::Args, registration: RegistrationId) {
    let Ok(Some(probe)) = tokio::time::timeout(
        Duration::from_secs(5),
        factory.take_local_mcp_probe(args.id, registration),
    )
    .await
    .unwrap_or_else(|_| Ok(None)) else {
        return;
    };
    let status = test_program(args.opencode_registration.as_deref(), &probe.command).await;
    let _ = factory
        .finish_local_mcp_probe(
            args.id,
            registration,
            &LocalMcpProbeView {
                id: probe.id,
                status,
            },
        )
        .await;
}

async fn test_program(registration: Option<&Path>, command: &[String]) -> ProbeStatus {
    if !executable(command) {
        return ProbeStatus::ProgramMissing;
    }
    let Some(registration) = registration else {
        return ProbeStatus::Unavailable;
    };
    let Ok(client) = OpenCodeClient::from_registration(registration) else {
        return ProbeStatus::Unavailable;
    };
    let Some(parent) = registration.parent() else {
        return ProbeStatus::Unavailable;
    };
    let Ok(temp) = tempfile::Builder::new()
        .prefix("mcp-probe-")
        .tempdir_in(parent)
    else {
        return ProbeStatus::Unavailable;
    };
    let Some(directory) = temp.path().to_str() else {
        return ProbeStatus::Unavailable;
    };
    if crate::managed_opencode::validate_workspace(temp.path()).is_err() {
        return ProbeStatus::Unavailable;
    }
    let result = tokio::time::timeout(Duration::from_secs(14), async {
        client
            .connect_local_mcp(directory, "yf-local-probe", command)
            .await?;
        loop {
            let status = client.local_mcp_statuses(directory).await?;
            match status.get("yf-local-probe").map(String::as_str) {
                Some("connected") => return Ok(ProbeStatus::Connected),
                Some("pending") => tokio::time::sleep(Duration::from_millis(150)).await,
                _ => return Ok(ProbeStatus::Unavailable),
            }
        }
    })
    .await;
    client.remove_local_mcp_probe(directory).await;
    let status: anyhow::Result<_> = result.unwrap_or(Ok(ProbeStatus::Unavailable));
    status.unwrap_or(ProbeStatus::Unavailable)
}

fn executable(command: &[String]) -> bool {
    let executable = Path::new(command.first().map_or("", String::as_str));
    executable.is_absolute()
        && factory_platform::is_executable(executable)
}

pub async fn prepare_remote(
    client: &OpenCodeClient,
    factory: &FactoryClient,
    registration: RegistrationId,
    command: &RunnerCommand,
    directory: &str,
    names: &[String],
    retry: bool,
    replace: bool,
) -> Result<Option<String>> {
    let states = client.local_mcp_statuses(directory).await?;
    let mcps = &command
        .agent_spec
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing agent spec"))?
        .remote_mcps;
    for (mcp, name) in mcps.iter().zip(names) {
        let state = states.get(name).map(String::as_str);
        // Revalidate even a connected transport before authorizing a new prompt;
        // a grant or credential revision may have changed during startup.
        if !replace && state == Some("connected") {
            factory
                .remote_mcp_lease(command.runner_id, registration, command.id, &mcp.id)
                .await?;
            continue;
        }
        if !retry || (!replace && state == Some("pending")) {
            return Ok(Some(mcp.name.clone()));
        }
        let (url, token) = factory
            .remote_mcp_lease(command.runner_id, registration, command.id, &mcp.id)
            .await?;
        client
            .connect_remote_mcp(directory, name, &url, &token)
            .await?;
    }
    if !names.is_empty() {
        let statuses = client.local_mcp_statuses(directory).await?;
        if let Some(mcp) = mcps.iter().zip(names).find_map(|(mcp, name)| {
            (statuses.get(name).map(String::as_str) != Some("connected")).then(|| mcp.name.clone())
        }) {
            return Ok(Some(mcp));
        }
    }
    Ok(None)
}

/// Returns the first incomplete connection. No tools are invoked here.
pub async fn prepare(
    client: &OpenCodeClient,
    directory: &str,
    mcps: &[LocalMcp],
    names: &[String],
    reconnect: bool,
) -> Result<Option<String>> {
    let states = client.local_mcp_statuses(directory).await?;
    ensure!(
        states.keys().all(|name| names.contains(name)),
        "OpenCode meldet nicht freigegebene MCPs in dieser Location"
    );
    for (mcp, name) in mcps.iter().zip(names) {
        let state = states.get(name).map(String::as_str);
        if state == Some("connected") {
            continue;
        }
        if reconnect && state != Some("pending") {
            if executable(&mcp.command) {
                client
                    .connect_local_mcp(directory, name, &mcp.command)
                    .await?;
            }
        }
        return Ok(Some(mcp.name.clone()));
    }
    Ok(None)
}
