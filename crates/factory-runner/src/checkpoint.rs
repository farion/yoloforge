//! Snapshot to independent Git objects/ref and bundle; never mutate HEAD, index or worktree.
use crate::{
    journal::{Journal, Record},
    workspace::{self, git, git_env, git_path},
    Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    spec: CheckpointSpec,
    workspace: Workspace,
    directory: PathBuf,
    parent: String,
    tree: String,
    commit: String,
    artifact: Option<GitCheckpoint>,
}
pub async fn execute(
    journal: &Journal,
    record: &mut Record,
    args: &Args,
    spec: &CheckpointSpec,
) -> CommandOutput {
    let checked = async {
        ensure!(
            spec.command_id == record.command.id,
            "Foreign checkpoint context"
        );
        for proof in &spec.stopped_runtimes {
            crate::containment::verify_removed(args, record.command.stream_id, proof).await?;
        }
        tokio::task::block_in_place(|| capture(journal, record, args, spec))
    }
    .await;
    match checked {
        Ok(checkpoint) => CommandOutput::Checkpoint { checkpoint },
        Err(e) => CommandOutput::CheckpointError {
            checkpoint_error: e.to_string().chars().take(240).collect(),
        },
    }
}
fn digest(path: &Path) -> Result<(String, u64)> {
    ensure!(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "Artifact is a symlink"
    );
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut bytes = [0u8; 65536];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
        size += count as u64;
    }
    Ok((format!("{:x}", hash.finalize()), size))
}
pub fn bundle_bytes(artifact: &GitCheckpoint) -> Result<Vec<u8>> {
    ensure!(
        artifact.bundle_bytes <= CHECKPOINT_BUNDLE_LIMIT as u64,
        "Checkpoint bundle exceeds 64 MiB transfer limit"
    );
    let path = Path::new(&artifact.bundle_path);
    let (sha, size) = digest(path)?;
    ensure!(
        sha == artifact.bundle_sha256 && size == artifact.bundle_bytes,
        "Checkpoint bundle modified before upload"
    );
    let bytes = fs::read(path)?;
    ensure!(
        bytes.len() as u64 == size && format!("{:x}", Sha256::digest(&bytes)) == sha,
        "Checkpoint bundle changed during read"
    );
    Ok(bytes)
}
pub fn install_bundle(root: &Path, import: &CheckpointImport, bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() <= CHECKPOINT_BUNDLE_LIMIT
            && bytes.len() as u64 == import.bundle_bytes
            && format!("{:x}", Sha256::digest(bytes)) == import.bundle_sha256,
        "Downloaded checkpoint digest differs"
    );
    fs::create_dir_all(root)?;
    let inbox = fs::canonicalize(root)?.join(".checkpoint-inbox");
    factory_platform::private_dir(&inbox)?;
    ensure!(
        !fs::symlink_metadata(&inbox)?.file_type().is_symlink(),
        "Checkpoint inbox is a symlink"
    );
    persist_bytes(&inbox.join(format!("{}.bundle", import.command_id)), bytes)
}
pub fn import_objects(repository: &Path, root: &Path, import: &CheckpointImport) -> Result<String> {
    let bundle = root
        .join(".checkpoint-inbox")
        .join(format!("{}.bundle", import.command_id));
    let (sha, size) = digest(&bundle)?;
    ensure!(
        sha == import.bundle_sha256 && size == import.bundle_bytes,
        "Imported checkpoint digest differs"
    );
    let reference = format!("refs/yoloforge/checkpoints/{}", import.command_id);
    let file = git_path(&bundle)?;
    ensure!(
        git(repository, &["bundle", "list-heads", &file])?
            == format!("{} {}", import.commit, reference),
        "Bundle does not contain the pinned checkpoint ref"
    );
    git(repository, &["bundle", "verify", &file])?;
    git(
        repository,
        &[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-auto-maintenance",
            &file,
            &reference,
        ],
    )?;
    ensure!(
        git(
            repository,
            &[
                "rev-parse",
                "--verify",
                &format!("{}^{{commit}}", import.commit)
            ]
        )? == import.commit,
        "Imported commit differs"
    );
    Ok(import.commit.clone())
}
fn persist_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        ensure!(
            fs::symlink_metadata(path)?.is_file()
                && !fs::symlink_metadata(path)?.file_type().is_symlink()
                && fs::read(path)? == bytes,
            "Existing manifest differs; refusing overwrite"
        );
        return Ok(());
    }
    let parent = path.parent().context("Artifact parent missing")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(path)?;
    factory_platform::sync_dir(parent)?;
    Ok(())
}
fn capture(
    journal: &Journal,
    record: &mut Record,
    args: &Args,
    spec: &CheckpointSpec,
) -> Result<GitCheckpoint> {
    let root = args
        .workspace_root
        .as_deref()
        .context("Workspace root missing")?;
    let work = workspace::existing(journal, &record.command, args.repositories.as_deref(), root)?;
    let path = Path::new(&work.path);
    let mut plan = if let Some(plan) = &record.checkpoint {
        ensure!(
            plan.spec == *spec && plan.workspace == work,
            "Checkpoint identity changed"
        );
        plan.clone()
    } else {
        ensure!(
            git(path, &["ls-files", "--unmerged"])?.is_empty(),
            "Resolve index conflicts before checkpointing"
        );
        ensure!(
            !git(path, &["ls-files", "--stage"])?
                .lines()
                .any(|l| l.starts_with("160000 ")),
            "Submodules require separate checkpoints"
        );
        let parent = git(path, &["rev-parse", "--verify", "HEAD^{commit}"])?;
        let temp = tempfile::tempdir()?;
        let index = temp.path().join("index");
        let index = git_path(&index)?;
        let env = [("GIT_INDEX_FILE", index.as_ref())];
        git_env(path, &["read-tree", &parent], &env)?;
        git_env(path, &["add", "--all", "--", "."], &env)?;
        ensure!(
            !git_env(path, &["ls-files", "--stage"], &env)?
                .lines()
                .any(|l| l.starts_with("160000 ")),
            "Nested repositories require separate checkpoints"
        );
        let tree = git_env(path, &["write-tree"], &env)?;
        // Detect changes during capture without touching the user's index.
        git_env(path, &["add", "--all", "--", "."], &env)?;
        ensure!(
            git_env(path, &["write-tree"], &env)? == tree
                && git(path, &["rev-parse", "HEAD"])? == parent,
            "Workspace changed during capture"
        );
        let date = format!("@{} +0000", record.command.created_unix_ms / 1000);
        let message = format!("YoloForge checkpoint {}", record.command.id);
        let commit = git_env(
            path,
            &["commit-tree", &tree, "-p", &parent, "-m", &message],
            &[
                ("GIT_AUTHOR_NAME", "YoloForge"),
                ("GIT_AUTHOR_EMAIL", "checkpoint@yoloforge.invalid"),
                ("GIT_COMMITTER_NAME", "YoloForge"),
                ("GIT_COMMITTER_EMAIL", "checkpoint@yoloforge.invalid"),
                ("GIT_AUTHOR_DATE", &date),
                ("GIT_COMMITTER_DATE", &date),
            ],
        )?;
        let directory = fs::canonicalize(root)?
            .join(".checkpoints")
            .join(record.command.id.to_string());
        ensure!(
            fs::symlink_metadata(&directory).is_err(),
            "Unjournaled checkpoint directory exists"
        );
        let plan = Plan {
            spec: spec.clone(),
            workspace: work.clone(),
            directory,
            parent,
            tree,
            commit,
            artifact: None,
        };
        record.checkpoint = Some(plan.clone());
        journal.save(record)?;
        plan
    };
    let reference = format!("refs/yoloforge/checkpoints/{}", record.command.id);
    let current = git(
        path,
        &["for-each-ref", "--format=%(objectname)", &reference],
    )?;
    if current.is_empty() {
        git(
            path,
            &[
                "update-ref",
                &reference,
                &plan.commit,
                &"0".repeat(plan.commit.len()),
            ],
        )?;
    } else {
        ensure!(
            current == plan.commit,
            "Checkpoint ref changed; refusing overwrite"
        );
    }
    let parent = plan.directory.parent().unwrap();
    fs::create_dir_all(parent)?;
    ensure!(
        !fs::symlink_metadata(parent)?.file_type().is_symlink(),
        "Checkpoint parent is a symlink"
    );
    factory_platform::private_dir(&plan.directory)?;
    ensure!(
        !fs::symlink_metadata(&plan.directory)?
            .file_type()
            .is_symlink(),
        "Checkpoint directory is a symlink"
    );
    let bundle = plan.directory.join("checkpoint.bundle");
    let manifest = plan.directory.join("manifest.json");
    if fs::symlink_metadata(&bundle).is_err() {
        let temp = tempfile::tempdir_in(&plan.directory)?;
        let temp_bundle = temp.path().join("bundle");
        let git_bundle = git_path(&temp_bundle)?;
        git(path, &["bundle", "create", &git_bundle, &reference])?;
        git(path, &["bundle", "verify", &git_bundle])?;
        let (sha, size) = digest(&temp_bundle)?;
        ensure!(
            size <= CHECKPOINT_BUNDLE_LIMIT as u64,
            "Checkpoint bundle exceeds 64 MiB transfer limit"
        );
        let artifact = GitCheckpoint {
            commit: plan.commit.clone(),
            parent: plan.parent.clone(),
            tree: plan.tree.clone(),
            bundle_path: bundle.to_string_lossy().into(),
            bundle_sha256: sha,
            bundle_bytes: size,
            manifest_path: manifest.to_string_lossy().into(),
        };
        if let Some(previous) = &plan.artifact {
            ensure!(
                previous == &artifact,
                "Rebuilt bundle differs from persisted intent"
            );
        }
        plan.artifact = Some(artifact);
        record.checkpoint = Some(plan.clone());
        journal.save(record)?;
        // FlushFileBuffers on Windows requires a handle with write access.
        fs::OpenOptions::new()
            .write(true)
            .open(&temp_bundle)?
            .sync_all()?;
        fs::hard_link(&temp_bundle, &bundle)?;
        factory_platform::sync_dir(&plan.directory)?;
    }
    let artifact = plan
        .artifact
        .context("Unjournaled bundle exists; refusing adoption")?;
    let (sha, size) = digest(&bundle)?;
    ensure!(
        sha == artifact.bundle_sha256 && size == artifact.bundle_bytes,
        "Checkpoint bundle modified"
    );
    let manifest_bytes = serde_json::to_vec_pretty(
        &serde_json::json!({"format":"yoloforge-checkpoint-v1","command":record.command.id,"stream":record.command.stream_id,"runner":record.command.runner_id,"ownership_epoch":record.command.ownership_epoch,"checkpoint":artifact,"context":spec}),
    )?;
    persist_bytes(&manifest, &manifest_bytes)?;
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn dirty_snapshot_preserves_index_and_worktree_imports_and_replays_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repository");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]).unwrap();
        git(&repo, &["config", "user.name", "Test"]).unwrap();
        git(&repo, &["config", "user.email", "test@example.invalid"]).unwrap();
        fs::write(repo.join("tracked"), "initial").unwrap();
        fs::write(repo.join("deleted"), "remove me").unwrap();
        fs::write(repo.join(".gitignore"), "ignored\n").unwrap();
        git(&repo, &["add", "."]).unwrap();
        git(&repo, &["commit", "-m", "initial"]).unwrap();
        let runner = RunnerId::new();
        let stream = StreamId::new();
        let project = ProjectId::new();
        let mapping = dir.path().join("mapping.json");
        fs::write(
            &mapping,
            serde_json::json!({project.to_string():repo}).to_string(),
        )
        .unwrap();
        let root = dir.path().join("worktrees");
        let db = dir.path().join("journal");
        let journal = Journal::open(&db, "http://localhost:7432", runner).unwrap();
        let mut cmd = RunnerCommand {
            id: CommandId::new(),
            stream_id: stream,
            runner_id: runner,
            ownership_epoch: 1,
            kind: CommandKind::PrepareWorkspace,
            state: CommandState::Accepted,
            result: None,
            workspace_spec: Some(WorkspaceSpec {
                target_git: None,
                setup: None,
                checkpoint: None,
                project_id: project,
                branch: format!("factory/{stream}"),
                base_branch: "main".into(),
            }),
            agent_spec: None,
            agent_progress: None,
            created_unix_ms: 1770000000000,
        };
        let CommandOutput::Workspace { workspace: work } =
            workspace::prepare(&journal, &cmd, Some(&mapping), &root)
        else {
            panic!("prepare failed")
        };
        let path = Path::new(&work.path);
        fs::write(path.join("tracked"), "staged version").unwrap();
        git(path, &["add", "tracked"]).unwrap();
        fs::write(path.join("tracked"), "working version").unwrap();
        fs::remove_file(path.join("deleted")).unwrap();
        fs::write(path.join("untracked-ü"), [0, 1, 2, 255]).unwrap();
        fs::write(path.join("ignored"), "do not archive").unwrap();
        crate::test_symlink_file("tracked", path.join("link")).unwrap();
        let head = git(path, &["rev-parse", "HEAD"]).unwrap();
        let status = git(path, &["status", "--porcelain=v1"]).unwrap();
        let staged = git(path, &["diff", "--cached", "--binary"]).unwrap();
        cmd.id = CommandId::new();
        cmd.kind = CommandKind::Checkpoint;
        let mut record = journal.receive(cmd.clone()).unwrap();
        let args = Args::try_parse_from([
            "factory-runner",
            "--id",
            &runner.to_string(),
            "--name",
            "test",
            "--repositories",
            mapping.to_str().unwrap(),
            "--workspace-root",
            root.to_str().unwrap(),
        ])
        .unwrap();
        let spec = CheckpointSpec {
            command_id: cmd.id,
            source_commands: vec![],
            stopped_runtimes: vec![],
            context: "test context".into(),
        };
        let result = capture(&journal, &mut record, &args, &spec).unwrap();
        assert_eq!(git(path, &["rev-parse", "HEAD"]).unwrap(), head);
        assert_eq!(git(path, &["status", "--porcelain=v1"]).unwrap(), status);
        assert_eq!(
            git(path, &["diff", "--cached", "--binary"]).unwrap(),
            staged
        );
        let imported = dir.path().join("imported");
        fs::create_dir(&imported).unwrap();
        git(&imported, &["init"]).unwrap();
        git(
            &imported,
            &[
                "fetch",
                &git_path(Path::new(&result.bundle_path)).unwrap(),
                &format!("refs/yoloforge/checkpoints/{}", cmd.id),
            ],
        )
        .unwrap();
        git(&imported, &["checkout", "--detach", &result.commit]).unwrap();
        assert_eq!(
            fs::read_to_string(imported.join("tracked")).unwrap(),
            "working version"
        );
        assert_eq!(
            fs::read(imported.join("untracked-ü")).unwrap(),
            vec![0, 1, 2, 255]
        );
        assert_eq!(
            fs::read_link(imported.join("link")).unwrap(),
            Path::new("tracked")
        );
        assert!(!imported.join("deleted").exists());
        assert!(!imported.join("ignored").exists());
        // Replay must retain the pinned tree even if a human edits the worktree later.
        fs::write(path.join("tracked"), "later user edit").unwrap();
        drop(journal);
        let journal = Journal::open(&db, "http://localhost:7432", runner).unwrap();
        let mut record = journal.record(cmd.id).unwrap().unwrap();
        assert_eq!(
            capture(&journal, &mut record, &args, &spec).unwrap(),
            result
        );
        // Recover a crash after saving artifact intent but before publishing the bundle.
        fs::remove_file(&result.bundle_path).unwrap();
        assert_eq!(
            capture(&journal, &mut record, &args, &spec).unwrap(),
            result
        );
        fs::write(&result.bundle_path, "foreign artifact").unwrap();
        assert!(capture(&journal, &mut record, &args, &spec).is_err());
        assert_eq!(
            fs::read_to_string(&result.bundle_path).unwrap(),
            "foreign artifact"
        );
        assert_eq!(
            fs::read_to_string(path.join("tracked")).unwrap(),
            "later user edit"
        );
    }
}
