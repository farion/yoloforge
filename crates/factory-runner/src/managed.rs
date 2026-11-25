//! Factory-owned repositories. Never reuse an unjournaled directory or delete failed attempts.
use crate::{
    journal::{Journal, Record},
    workspace::git,
    Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    source: String,
    branch: String,
    path: PathBuf,
    attempted: bool,
    ready: bool,
    #[serde(default)]
    create_local: bool,
}
fn directory(root: &Path, id: ProjectId) -> PathBuf {
    root.join(".repositories").join(id.to_string())
}
pub fn resolve(spec: &WorkspaceSpec, root: &Path) -> Result<PathBuf> {
    let setup = spec.setup.as_ref().context("Setup missing")?;
    let dir = directory(&fs::canonicalize(root)?, spec.project_id);
    ensure!(
        fs::canonicalize(&dir)? == dir,
        "Managed repository directory replaced"
    );
    let binding = dir.join("binding.json");
    ensure!(
        !fs::symlink_metadata(&binding)?.file_type().is_symlink(),
        "Managed binding replaced"
    );
    let plan: Plan = serde_json::from_slice(&fs::read(binding)?)?;
    ensure!(
        plan.ready && plan.source == setup.repository_source && plan.branch == spec.base_branch,
        "Managed repository binding differs"
    );
    ensure!(
        plan.path.parent() == Some(dir.as_path()) && fs::canonicalize(&plan.path)? == plan.path,
        "Managed repository path replaced"
    );
    ensure!(
        git(&plan.path, &["rev-parse", "--is-bare-repository"])? == "true",
        "Managed repository identity differs"
    );
    Ok(plan.path)
}
fn save(j: &Journal, r: &mut Record, p: &Plan) -> Result<()> {
    r.repository = Some(p.clone());
    tokio::task::block_in_place(|| j.save(r))
}
pub async fn ensure_repository(j: &Journal, r: &mut Record, args: &Args) -> Result<()> {
    let spec = r
        .command
        .workspace_spec
        .clone()
        .context("Workspace specification missing")?;
    let Some(setup) = &spec.setup else {
        return Ok(());
    };
    if !setup.repository_path.is_empty() {
        return Ok(());
    }
    let root = args
        .workspace_root
        .as_ref()
        .context("Workspace root missing")?;
    fs::create_dir_all(root)?;
    let root = fs::canonicalize(root)?;
    let dir = directory(&root, spec.project_id);
    fs::create_dir_all(&dir)?;
    ensure!(
        fs::canonicalize(&dir)? == dir,
        "Managed directory is a symlink"
    );
    if fs::symlink_metadata(dir.join("binding.json")).is_ok() {
        resolve(&spec, &root)?;
        return Ok(());
    }
    let mut plan = if let Some(plan) = &r.repository {
        ensure!(
            plan.source == setup.repository_source
                && plan.branch == spec.base_branch
                && plan.path == dir.join(r.command.id.to_string()),
            "Repository plan changed"
        );
        plan.clone()
    } else {
        let plan = Plan {
            source: setup.repository_source.clone(),
            branch: spec.base_branch.clone(),
            path: dir.join(r.command.id.to_string()),
            attempted: false,
            ready: false,
            create_local: Path::new(&setup.repository_source).is_absolute()
                && matches!(fs::symlink_metadata(&setup.repository_source),Err(e) if e.kind()==std::io::ErrorKind::NotFound),
        };
        save(j, r, &plan)?;
        plan
    };
    if !plan.ready {
        if plan.create_local {
            tokio::task::block_in_place(|| {
                initialize_local(&plan.source, &plan.branch, spec.project_id)
            })?;
        }
        ensure!(!plan.attempted,"Repository creation outcome unconfirmed; retry preparation as a new command (previous attempt retained)");
        fs::create_dir(&plan.path)
            .context("Managed repository destination already occupied; nothing overwritten")?;
        plan.attempted = true;
        save(j, r, &plan)?;
        if plan.source == "factory:managed" {
            tokio::task::block_in_place(|| -> Result<()> {
                git(
                    &plan.path,
                    &["init", "--bare", "--initial-branch", &plan.branch],
                )?;
                let tree = git(&plan.path, &["mktree"])?;
                let commit = crate::workspace::git_env(
                    &plan.path,
                    &["commit-tree", &tree, "-m", "Initialize Factory project"],
                    &[
                        ("GIT_AUTHOR_NAME", "YoloForge"),
                        ("GIT_AUTHOR_EMAIL", "factory@yoloforge.invalid"),
                        ("GIT_COMMITTER_NAME", "YoloForge"),
                        ("GIT_COMMITTER_EMAIL", "factory@yoloforge.invalid"),
                    ],
                )?;
                git(
                    &plan.path,
                    &[
                        "update-ref",
                        &format!("refs/heads/{}", plan.branch),
                        &commit,
                        &"0".repeat(commit.len()),
                    ],
                )?;
                Ok(())
            })?;
        } else {
            ensure!(
                !plan.source.is_empty() && !plan.source.starts_with('-'),
                "Invalid repository source"
            );
            let mut command = tokio::process::Command::new("git");
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("GIT_") {
                    command.env_remove(key);
                }
            }
            command
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "clone",
                    "--bare",
                    "--no-hardlinks",
                    "--branch",
                    &plan.branch,
                    "--",
                    &plan.source,
                ])
                .arg(&plan.path)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            let status =
                tokio::time::timeout(std::time::Duration::from_secs(120), command.status())
                    .await
                    .context(
                        "Clone timed out; attempt retained, retry preparation as a new command",
                    )??;
            ensure!(status.success(),"Repository import failed; check runner Git access. Retry preparation after fixing access; no files deleted");
        }
        tokio::task::block_in_place(|| {
            git(
                &plan.path,
                &[
                    "rev-parse",
                    "--verify",
                    &format!("refs/heads/{}^{{commit}}", plan.branch),
                ],
            )
        })?;
        plan.ready = true;
        save(j, r, &plan)?;
    }
    let mut file = tempfile::NamedTempFile::new_in(&dir)?;
    file.write_all(&serde_json::to_vec(&plan)?)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(dir.join("binding.json"))
        .map_err(|_| anyhow::anyhow!("Managed binding occupied; nothing overwritten"))?;
    factory_platform::sync_dir(&dir)?;
    resolve(&spec, &root)?;
    Ok(())
}
// Publish only a complete repository, without replacing an entry created by another process.
fn initialize_local(source: &str, branch: &str, project: ProjectId) -> Result<()> {
    let target = Path::new(source);
    if fs::symlink_metadata(target).is_ok() {
        ensure!(
            !fs::symlink_metadata(target)?.file_type().is_symlink(),
            "Project folder was replaced by a symlink"
        );
        ensure!(
            git(target, &["config", "--local", "--get", "factory.projectId"])?
                == project.to_string(),
            "Project folder was occupied during creation; existing files preserved"
        );
        git(
            target,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{branch}^{{commit}}"),
            ],
        )?;
        return Ok(());
    }
    let parent = target.parent().context("Project folder has no parent")?;
    fs::create_dir_all(parent).context("Cannot create project parent folder")?;
    let attempt = tempfile::Builder::new()
        .prefix(".yoloforge-create-")
        .tempdir_in(parent)?
        .keep();
    git(&attempt, &["init", "--initial-branch", branch])?;
    git(
        &attempt,
        &[
            "config",
            "--local",
            "factory.projectId",
            &project.to_string(),
        ],
    )?;
    crate::workspace::git_env(
        &attempt,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "Initialize Factory project",
        ],
        &[
            ("GIT_AUTHOR_NAME", "YoloForge"),
            ("GIT_AUTHOR_EMAIL", "factory@yoloforge.invalid"),
            ("GIT_COMMITTER_NAME", "YoloForge"),
            ("GIT_COMMITTER_EMAIL", "factory@yoloforge.invalid"),
        ],
    )?;
    factory_platform::rename_noreplace(&attempt, target)
    .context("Project folder occupied during creation; prepared repository retained")?;
    factory_platform::sync_dir(parent)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    async fn exercise(import: bool, local: bool) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        let path = dir.path().join("journal");
        let runner = RunnerId::new();
        let project = ProjectId::new();
        let stream = StreamId::new();
        let source = if local {
            dir.path()
                .join("new-parent/new-project")
                .to_string_lossy()
                .into_owned()
        } else if import {
            let repo = dir.path().join("source");
            fs::create_dir(&repo).unwrap();
            git(&repo, &["init", "-b", "main"]).unwrap();
            git(
                &repo,
                &[
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "commit",
                    "--allow-empty",
                    "-m",
                    "initial",
                ],
            )
            .unwrap();
            fs::write(repo.join("user-draft"), "keep").unwrap();
            repo.to_string_lossy().into_owned()
        } else {
            "factory:managed".into()
        };
        let args = Args::parse_from([
            "runner",
            "--id",
            &runner.to_string(),
            "--name",
            "test",
            "--workspace-root",
            root.to_str().unwrap(),
            "--journal",
            path.to_str().unwrap(),
        ]);
        let c:RunnerCommand=serde_json::from_value(serde_json::json!({"id":CommandId::new(),"stream_id":stream,"runner_id":runner,"ownership_epoch":1,"kind":"prepare_workspace","state":"accepted","created_unix_ms":1,"workspace_spec":{"project_id":project,"branch":format!("factory/{stream}"),"base_branch":"main","setup":{"runner_id":runner,"repository_source":source,"repository_path":"","service_registration":"","harness":null}}})).unwrap();
        let j = Journal::open(&path, &args.server, runner).unwrap();
        let mut r = j.receive(c.clone()).unwrap();
        ensure_repository(&j, &mut r, &args).await.unwrap();
        if local {
            assert!(Path::new(&source).join(".git").is_dir());
        }
        let resolved = resolve(c.workspace_spec.as_ref().unwrap(), &root).unwrap();
        let head = git(&resolved, &["rev-parse", "main"]).unwrap();
        let CommandOutput::Workspace { workspace } = crate::workspace::prepare(&j, &c, None, &root)
        else {
            panic!("managed workspace")
        };
        fs::write(Path::new(&workspace.path).join("draft"), "user").unwrap();
        drop(j);
        let j = Journal::open(&path, &args.server, runner).unwrap();
        let mut r = j.receive(c.clone()).unwrap();
        ensure_repository(&j, &mut r, &args).await.unwrap();
        assert_eq!(git(&resolved, &["rev-parse", "main"]).unwrap(), head);
        assert_eq!(
            fs::read_to_string(Path::new(&workspace.path).join("draft")).unwrap(),
            "user"
        );
        let mut other = c.clone();
        other.id = CommandId::new();
        other
            .workspace_spec
            .as_mut()
            .unwrap()
            .setup
            .as_mut()
            .unwrap()
            .repository_source = "different".into();
        let mut record = j.receive(other).unwrap();
        assert!(ensure_repository(&j, &mut record, &args).await.is_err());
        if import {
            assert_eq!(
                fs::read_to_string(Path::new(&source).join("user-draft")).unwrap(),
                "keep"
            );
            assert!(!Path::new(&workspace.path).join("user-draft").exists());
        }
        let mut occupied = c;
        occupied.id = CommandId::new();
        occupied.workspace_spec.as_mut().unwrap().project_id = ProjectId::new();
        let target = directory(&root, occupied.workspace_spec.as_ref().unwrap().project_id)
            .join(occupied.id.to_string());
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("existing"), "keep").unwrap();
        let mut record = j.receive(occupied).unwrap();
        assert!(ensure_repository(&j, &mut record, &args).await.is_err());
        assert_eq!(fs::read_to_string(target.join("existing")).unwrap(), "keep");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn empty_project_manages_its_own_repository_and_preserves_work_after_restart() {
        exercise(false, false).await;
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn import_preserves_source_and_refuses_occupied_or_rebound_destinations() {
        exercise(true, false).await;
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn requested_missing_folder_is_created_and_recovers_after_restart() {
        exercise(false, true).await;
    }
    #[test]
    fn local_initialization_reconciles_its_own_folder_and_never_adopts_occupied_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project");
        let project = ProjectId::new();
        initialize_local(path.to_str().unwrap(), "main", project).unwrap();
        fs::write(path.join("draft"), "preserve").unwrap();
        initialize_local(path.to_str().unwrap(), "main", project).unwrap();
        assert!(initialize_local(path.to_str().unwrap(), "main", ProjectId::new()).is_err());
        assert_eq!(fs::read_to_string(path.join("draft")).unwrap(), "preserve");
        let occupied = dir.path().join("occupied");
        fs::create_dir(&occupied).unwrap();
        fs::write(occupied.join("file"), "user").unwrap();
        assert!(initialize_local(occupied.to_str().unwrap(), "main", project).is_err());
        assert!(!occupied.join(".git").exists());
    }
}
