use crate::{journal::Journal, Args};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_opencode::OpenCodeClient;
use factory_protocol::{client::FactoryClient, DraftReset};

pub(crate) async fn step(client: &FactoryClient, journal: &Journal, args: &Args, registration: RegistrationId) -> Result<()> {
    let Some(reset) = client.pending_draft_reset(args.id, registration).await? else { return Ok(()); };
    discard(journal, args, &reset).await?;
    client.confirm_draft_reset(args.id, registration, reset.request_id).await?;
    Ok(())
}

async fn discard(journal: &Journal, args: &Args, reset: &DraftReset) -> Result<()> {
    ensure!(reset.merger_commands.is_empty(), "Cannot discard a draft with separate Merger sessions");
    let mut record = tokio::task::block_in_place(|| journal.record(reset.command_id))?.context("Draft reset execution is not journaled")?;
    let command = &record.command;
    ensure!(record.reported && !record.recovery_required && record.archive.finished
        && record.archive.blobs.iter().all(|b| b.offset == b.file.bytes)
        && command.runner_id == args.id && command.stream_id == reset.stream_id
        && command.session_root() == reset.root_id && command.ownership_epoch == reset.ownership_epoch
        && command.kind == CommandKind::RunAgent && !command.requires_target_git()
        && command.agent_spec.as_ref().is_some_and(|s| s.role.is_empty() && !s.read_only)
        && matches!(&record.result, Some(CommandOutput::Agent { agent }) if agent.phase == AgentPhase::Failed
            || (agent.phase == AgentPhase::Succeeded && publication_was_blocked(&agent.summary))),
        "Draft reset execution identity or archive state changed");
    ensure!(record.draft_reset_verified.is_none_or(|request| request == reset.request_id), "Draft reset request changed");
    let effective = crate::setup::effective(args, command)?;
    let execution = record.agent.as_ref().context("Draft reset has no OpenCode execution")?;
    let workspace = execution.workspace.clone();
    let root = effective.workspace_root.as_ref().context("Worktree root missing")?;
    let api = OpenCodeClient::from_registration(effective.opencode_registration.as_ref().context("OpenCode registration missing")?)?;
    ensure!(execution.runtime.is_none() && execution.endpoint == api.endpoint(), "OpenCode runtime or endpoint changed");
    let id = command.session_id();
    let verified = record.draft_reset_verified == Some(reset.request_id);
    tokio::task::block_in_place(|| crate::workspace::discard(journal, command, &workspace, root, verified, false))?;
    if let Some(session) = api.session(&id).await? {
        ensure!(session["id"] == id
            && session["metadata"]["factory_command"] == reset.root_id.to_string()
            && session["location"]["directory"] == workspace.path, "OpenCode session identity changed");
        ensure!(!api.active(&id).await?, "OpenCode session is active");
        for kind in ["inbox", "form", "permission"] {
            ensure!(api.session_items(&id, kind).await?.as_array().is_some_and(|v| v.is_empty()),
                "OpenCode session still has pending input");
        }
        record.draft_reset_verified = Some(reset.request_id);
        tokio::task::block_in_place(|| journal.save(&record))?;
        api.delete_session(&id).await?;
    } else {
        ensure!(verified, "OpenCode session missing before identity could be verified");
    }
    tokio::task::block_in_place(|| crate::workspace::discard(journal, &record.command, &workspace, root, true, true))?;
    Ok(())
}
