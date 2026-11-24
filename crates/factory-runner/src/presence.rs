//! Read-only, host-local evidence. Never infer service liveness from command state.
use crate::{journal::Journal, setup, Args};
use anyhow::Result;
use factory_core::*;
use factory_opencode::OpenCodeClient;
use factory_protocol::{client::FactoryClient, *};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
type ServiceSessions = (
    OpenCodeClient,
    Vec<(CommandId, String, String)>,
    Vec<CommandId>,
);

pub async fn sync(
    client: &FactoryClient,
    journal: &Journal,
    args: &Args,
    registration: RegistrationId,
) -> Result<()> {
    let snapshot = client.snapshot().await?;
    let mut latest = BTreeMap::new();
    for c in snapshot
        .commands
        .iter()
        .rev()
        .filter(|c| c.runner_id == args.id && c.kind == CommandKind::RunAgent)
    {
        if latest.contains_key(&c.session_root()) {
            continue;
        }
        if let Some(record) = journal.record(c.id)? {
            if record.agent.is_some() {
                latest.insert(c.session_root(), (c, record));
            }
        }
    }
    let mut services: BTreeMap<String, ServiceSessions> = BTreeMap::new();
    for (c, record) in latest.into_values() {
        let Some(execution) = record.agent else {
            continue;
        };
        let effective = setup::effective(args, &record.command)?;
        let Some(path) = effective.opencode_registration else {
            continue;
        };
        let Ok(oc) = OpenCodeClient::from_registration(&path) else {
            continue;
        };
        if oc.endpoint() != execution.endpoint {
            continue;
        }
        services
            .entry(execution.endpoint)
            .or_insert_with(|| (oc, vec![], vec![]))
            .1
            .push((c.id, c.session_id(), execution.workspace.path));
    }
    for c in snapshot.commands.iter().filter(|c| {
        c.runner_id == args.id
            && c.kind == CommandKind::PrepareWorkspace
            && c.state == CommandState::Succeeded
    }) {
        let effective = setup::effective(args, c)?;
        if let Some(path) = effective.opencode_registration {
            if let Ok(oc) = OpenCodeClient::from_registration(&path) {
                services
                    .entry(oc.endpoint().to_string())
                    .or_insert_with(|| (oc, vec![], vec![]))
                    .2
                    .push(c.id);
            }
        }
    }
    if let Some(path) = &args.opencode_registration {
        if let Ok(oc) = OpenCodeClient::from_registration(path) {
            services
                .entry(oc.endpoint().to_string())
                .or_insert((oc, vec![], vec![]));
        }
    }
    let mut instances = Vec::new();
    for (endpoint, (oc, commands, mut ids)) in services {
        if !matches!(oc.status().await, OpenCodeStatus::Ready { .. }) {
            continue;
        }
        for (id, session, workspace) in commands {
            if let Ok(Some(value)) = oc.session(&session).await {
                if value["id"] == session && value["location"]["directory"] == workspace {
                    ids.push(id);
                }
            }
        }
        // Endpoint never leaves the runner; hash is scoped to its identity.
        let id = format!(
            "{:x}",
            Sha256::digest(format!("{}/{endpoint}", args.id).as_bytes())
        );
        instances.push(OpenCodeInstance {
            id,
            runner_id: args.id,
            commands: ids,
            observed_unix_ms: 0,
        });
    }
    client
        .report_instances(args.id, registration, &ReportInstances { instances })
        .await?;
    Ok(())
}
