use crate::{journal::{Journal, Record}, workspace::{common_dir, git, git_path}, Args};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_opencode::OpenCodeClient;
use factory_protocol::{client::{ClientError, FactoryClient}, DraftReset};
use std::{fs, path::{Path, PathBuf}};

pub(crate) async fn step(client: &FactoryClient, journal: &Journal, args: &Args, registration: RegistrationId) -> Result<()> {
    let reset = match client.pending_draft_reset(args.id, registration).await {
        Ok(Some(reset)) => reset,
        Ok(None) => return Ok(()),
        Err(ClientError::Http { status, .. }) if status.as_u16() == 404 => return Ok(()), // older server
        Err(error) => return Err(error.into()),
    };
    discard(journal, args, &reset).await?;
    client.confirm_draft_reset(args.id, registration, reset.request_id).await?;
    Ok(())
}

async fn discard(journal: &Journal, args: &Args, reset: &DraftReset) -> Result<()> {
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
    if !reset.merger_commands.is_empty() {
        return discard_after_mergers(journal, &mut record, reset, root, &api).await;
    }
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

async fn discard_after_mergers(journal: &Journal, original: &mut Record, reset: &DraftReset, root: &Path, api: &OpenCodeClient) -> Result<()> {
    let verified = original.draft_reset_verified == Some(reset.request_id);
    let (root, repository, worktrees) = tokio::task::block_in_place(|| -> Result<_> {
        let root = fs::canonicalize(root)?;
        let workspace = &original.agent.as_ref().context("Missing original workspace")?.workspace;
        let repository = crate::workspace::draft_reset_repository(journal, &original.command, workspace, &root)?;
        let path = root.join(Path::new(&workspace.path).file_name().context("Missing original workspace name")?);
        let mut worktrees = vec![ResetWorktree {
            id: original.command.session_id(), owner: reset.root_id, path,
            branch: Some(workspace.branch.clone()), started: true,
        }];
        let mut seen = std::collections::BTreeSet::new();
        for id in &reset.merger_commands {
            ensure!(seen.insert(*id), "Repeated Merger in draft reset");
            let record = journal.record(*id)?.context("Merger is not journaled")?;
            let c = &record.command;
            let source = c.agent_spec.as_ref().filter(|s| s.role == MERGER_ROLE && !s.read_only)
                .and_then(|s| s.prompt.lines().next()).and_then(|l| l.strip_prefix(MERGE_PARENT_PREFIX))
                .and_then(|id| id.parse::<CommandId>().ok()).context("Missing Merger source")?;
            let source = journal.record(source)?.context("Merger source is not journaled")?;
            ensure!(record.reported && !record.recovery_required && c.runner_id == original.command.runner_id
                && c.stream_id == reset.stream_id && c.ownership_epoch == reset.ownership_epoch
                && c.id == *id && c.session_root() == *id && c.kind == CommandKind::RunAgent
                && source.command.session_root() == reset.root_id && source.command.stream_id == reset.stream_id
                && source.command.runner_id == c.runner_id && source.command.ownership_epoch == c.ownership_epoch,
                "Merger does not belong to the discarded session");
            let path = root.join(format!("m-{id}"));
            if let Some(agent) = &record.agent {
                ensure!(record.archive.finished && record.archive.blobs.iter().all(|b| b.offset == b.file.bytes)
                    && agent.runtime.is_none() && agent.endpoint == api.endpoint()
                    && git_path(&path)? == agent.workspace.path, "Merger runtime, path or archive is not ready");
            }
            worktrees.push(ResetWorktree { id: c.session_id(), owner: *id, path,
                branch: None, started: record.agent.is_some() });
        }
        Ok((root, repository, worktrees))
    })?;
    let backup = root.join(format!("discarded-{}", reset.request_id));
    let present = tokio::task::block_in_place(|| -> Result<_> {
        if exists(&backup)? {
            ensure!(verified && fs::canonicalize(&backup)? == backup
                && fs::symlink_metadata(&backup)?.file_type().is_dir(), "Draft reset backup location changed");
        }
        worktrees.iter().map(|tree| tree.location(&repository, &backup, verified)).collect::<Result<Vec<_>>>()
    })?;
    let mut sessions = Vec::with_capacity(worktrees.len());
    for tree in &worktrees {
        let session = api.session(&tree.id).await?;
        if let Some(session) = &session {
            ensure!(tree.started && session["id"] == tree.id
                && session["metadata"]["factory_command"] == tree.owner.to_string()
                && session["location"]["directory"] == git_path(&tree.path)?.as_ref(), "OpenCode session identity changed");
            ensure!(!api.active(&tree.id).await?, "OpenCode session is active");
            for kind in ["inbox", "form", "permission"] {
                ensure!(api.session_items(&tree.id, kind).await?.as_array().is_some_and(|v| v.is_empty()),
                    "OpenCode session still has pending input");
            }
        } else {
            ensure!(verified || !tree.started, "OpenCode session missing before identity could be verified");
        }
        sessions.push(session.is_some());
    }
    tokio::task::block_in_place(|| -> Result<()> {
        original.draft_reset_verified = Some(reset.request_id);
        journal.save(original)?;
        if !exists(&backup)? {
            let mut dir = fs::DirBuilder::new();
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut dir, 0o700);
            dir.create(&backup)?;
        }
        for (tree, location) in worktrees.iter().zip(present) {
            if location.as_deref() == Some(tree.path.as_path()) {
                let destination = tree.destination(&backup)?;
                git(&repository, &["worktree", "move", "--", &git_path(&tree.path)?, &git_path(&destination)?])?;
                ensure!(tree.location(&repository, &backup, true)?.as_ref() == Some(&destination),
                    "Draft reset backup could not be verified");
            }
        }
        Ok(())
    })?;
    for (tree, present) in worktrees.iter().zip(sessions) {
        if present { api.delete_session(&tree.id).await?; }
    }
    Ok(())
}

struct ResetWorktree {
    id: String,
    owner: CommandId,
    path: PathBuf,
    branch: Option<String>,
    started: bool,
}

impl ResetWorktree {
    fn destination(&self, backup: &Path) -> Result<PathBuf> {
        Ok(backup.join(self.path.file_name().context("Missing worktree name")?))
    }

    fn location(&self, repository: &Path, backup: &Path, verified: bool) -> Result<Option<PathBuf>> {
        let destination = self.destination(backup)?;
        let original = exists(&self.path)?;
        let saved = exists(&destination)?;
        let listed = git(repository, &["worktree", "list", "--porcelain", "-z"])?;
        let entry_at = |name: &str| listed.split("\0\0").find(|entry| entry.split('\0').next()
            .and_then(|line| line.strip_prefix("worktree ")) == Some(name));
        ensure!(!saved || (verified && !original), "Draft reset worktree backup conflicts with original");
        let path = if saved { destination } else if original { self.path.clone() } else {
            ensure!(!self.started && entry_at(&git_path(&self.path)?).is_none()
                && entry_at(&git_path(&destination)?).is_none(), "Draft reset worktree unexpectedly missing");
            return Ok(None);
        };
        ensure!(fs::symlink_metadata(&path)?.file_type().is_dir() && fs::canonicalize(&path)? == path
            && fs::symlink_metadata(path.join(".git"))?.file_type().is_file()
            && common_dir(&path)? == common_dir(repository)?
            && fs::canonicalize(git(&path, &["rev-parse", "--show-toplevel"])?)? == path,
            "Draft reset worktree identity changed");
        let path_name = git_path(&path)?;
        let entry = entry_at(&path_name)
            .context("Draft reset worktree not listed in the recorded repository")?;
        let head = self.branch.as_ref().map_or_else(|| "detached".into(), |branch| format!("branch refs/heads/{branch}"));
        ensure!(entry.split('\0').any(|line| line == head), "Draft reset worktree branch changed");
        Ok(Some(path))
    }
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarantine_keeps_merge_conflicts_and_untracked_files_and_can_resume() {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let repo = root.join("repo");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]).unwrap();
        git(&repo, &["config", "user.email", "test@example.test"]).unwrap();
        git(&repo, &["config", "user.name", "Test"]).unwrap();
        fs::write(repo.join("source"), "base\n").unwrap();
        git(&repo, &["add", "source"]).unwrap();
        git(&repo, &["commit", "-m", "base"]).unwrap();
        let base = git(&repo, &["rev-parse", "HEAD"]).unwrap();
        let id = CommandId::new();
        let tree = ResetWorktree {
            id: agent_session_id(id), owner: id, path: root.join(format!("m-{id}")), branch: None, started: true,
        };
        git(&repo, &["worktree", "add", "--detach", &git_path(&tree.path).unwrap(), &base]).unwrap();
        fs::write(tree.path.join("source"), "merger\n").unwrap();
        git(&tree.path, &["commit", "-am", "merger"]).unwrap();
        fs::write(repo.join("source"), "main\n").unwrap();
        git(&repo, &["commit", "-am", "main"]).unwrap();
        assert!(git(&tree.path, &["merge", "main"]).is_err());
        fs::write(tree.path.join("notes"), "untracked work\n").unwrap();
        let index = git(&tree.path, &["ls-files", "-u"]).unwrap();
        assert!(!index.is_empty());
        let backup = root.join("discarded");
        fs::create_dir(&backup).unwrap();
        let dest = tree.destination(&backup).unwrap();
        assert_eq!(tree.location(&repo, &backup, false).unwrap(), Some(tree.path.clone()));
        git(&repo, &["worktree", "move", "--", &git_path(&tree.path).unwrap(), &git_path(&dest).unwrap()]).unwrap();
        assert!(tree.location(&repo, &backup, false).is_err());
        assert_eq!(tree.location(&repo, &backup, true).unwrap(), Some(dest.clone()));
        assert_eq!(git(&dest, &["ls-files", "-u"]).unwrap(), index);
        assert_eq!(fs::read_to_string(dest.join("notes")).unwrap(), "untracked work\n");
        assert_eq!(fs::read_to_string(repo.join("source")).unwrap(), "main\n");
    }
}
