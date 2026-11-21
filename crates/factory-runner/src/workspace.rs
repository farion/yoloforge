//! Local Git worktrees with a durable pre-operation plan. Never reset, clean or delete.
use crate::journal::Journal;
use anyhow::{bail, ensure, Context, Result};
use factory_core::*;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Plan {
    spec: WorkspaceSpec,
    repository: PathBuf,
    common_dir: PathBuf,
    path: PathBuf,
    commit: String,
    ready: bool,
}

// Arguments are passed directly, never through a shell. Output goes to local
// temp files to avoid pipe deadlocks while enforcing a command timeout.
pub(crate) fn git(repository: &Path, args: &[&str]) -> Result<String> {
    git_env(repository, args, &[])
}
pub(crate) fn git_env(repository: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
    let out = tempfile::tempfile()?;
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    let mut child = command
        .arg("-C")
        .arg(repository)
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(
                status.success(),
                "Git operation failed; inspect repository/worktree before retrying"
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Git operation timed out; inspect repository/worktree before retrying");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    use std::io::{Read, Seek, SeekFrom};
    let mut out = out;
    out.seek(SeekFrom::Start(0))?;
    let mut text = String::new();
    out.take(1_048_577).read_to_string(&mut text)?;
    ensure!(
        text.len() <= 1_048_576,
        "Git output exceeds inspection limit"
    );
    Ok(text.trim().to_string())
}

fn common_dir(repository: &Path) -> Result<PathBuf> {
    Ok(fs::canonicalize(git(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?)?)
}
fn save(journal: &Journal, stream: impl ToString, plan: &Plan) -> Result<()> {
    journal.db.execute("INSERT INTO workspaces(stream_id,data) VALUES(?1,?2) ON CONFLICT(stream_id) DO UPDATE SET data=excluded.data",
        params![stream.to_string(),serde_json::to_string(plan)?])?;
    Ok(())
}
fn key(command: &RunnerCommand) -> String {
    if command.ownership_epoch == 1 {
        command.stream_id.to_string()
    } else {
        format!("{}-epoch-{}", command.stream_id, command.ownership_epoch)
    }
}

pub fn prepare(
    journal: &Journal,
    command: &RunnerCommand,
    mapping: Option<&Path>,
    root: &Path,
) -> CommandOutput {
    match prepare_inner(journal, command, mapping, root) {
        Ok(workspace) => CommandOutput::Workspace { workspace },
        // Errors here are ours, never Git stderr (which could contain credentials).
        Err(error) => CommandOutput::WorkspaceError {
            workspace_error: error.to_string().chars().take(240).collect(),
        },
    }
}

fn prepare_inner(
    journal: &Journal,
    command: &RunnerCommand,
    mapping: Option<&Path>,
    root: &Path,
) -> Result<Workspace> {
    let spec = command
        .workspace_spec
        .as_ref()
        .context("Workspace specification missing")?;
    ensure!(
        spec.branch
            == if command.ownership_epoch == 1 {
                format!("factory/{}", command.stream_id)
            } else {
                format!(
                    "factory/{}-epoch-{}",
                    command.stream_id, command.ownership_epoch
                )
            },
        "Unexpected stream branch"
    );
    let mapping =
        mapping.context("Configure --repositories with project IDs and local repository paths")?;
    let repositories: BTreeMap<ProjectId, PathBuf> =
        serde_json::from_slice(&fs::read(mapping).context("Cannot read repository mapping")?)
            .context("Invalid repository mapping")?;
    let repository = fs::canonicalize(
        repositories
            .get(&spec.project_id)
            .context("Project has no local repository mapping")?,
    )
    .context("Mapped repository does not exist")?;
    let common = common_dir(&repository).context("Mapped path is not a Git repository")?;
    fs::create_dir_all(root).context("Cannot create workspace root")?;
    let root = fs::canonicalize(root)?;
    let path = root.join(key(command));
    let previous: Option<String> = journal
        .db
        .query_row(
            "SELECT data FROM workspaces WHERE stream_id=?1",
            [key(command)],
            |r| r.get(0),
        )
        .optional()?;
    let reference = format!("refs/heads/{}", spec.branch);
    let mut plan: Plan = if let Some(previous) = previous {
        let plan: Plan = serde_json::from_str(&previous)?;
        ensure!(
            plan.spec == *spec
                && plan.repository == repository
                && plan.common_dir == common
                && plan.path == path,
            "Workspace mapping changed; restore original configuration before retrying"
        );
        plan
    } else {
        ensure!(
            !path.try_exists()? && fs::symlink_metadata(&path).is_err(),
            "Workspace path already exists; refusing to overwrite"
        );
        git(
            &repository,
            &[
                "check-ref-format",
                &format!("refs/heads/{}", spec.base_branch),
            ],
        )
        .context("Invalid base branch")?;
        let commit = if let Some(import) = &spec.checkpoint {
            crate::checkpoint::import_objects(&repository, &root, import)?
        } else {
            git(
                &repository,
                &[
                    "rev-parse",
                    "--verify",
                    &format!("refs/heads/{}^{{commit}}", spec.base_branch),
                ],
            )
            .context("Base branch does not exist locally")?
        };
        let refs = git(
            &repository,
            &["for-each-ref", "--format=%(refname)", &reference],
        )?;
        ensure!(
            refs.is_empty(),
            "Stream branch already exists without a journal plan"
        );
        let plan = Plan {
            spec: spec.clone(),
            repository: repository.clone(),
            common_dir: common,
            path: path.clone(),
            commit,
            ready: false,
        };
        save(journal, key(command), &plan)?;
        plan
    };
    if fs::symlink_metadata(&path).is_ok() {
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "Workspace path is a symlink"
        );
        ensure!(
            path.join(".git").is_file(),
            "Existing path is not the planned worktree; inspect manually"
        );
        ensure!(
            fs::canonicalize(git(&path, &["rev-parse", "--show-toplevel"])?)? == path
                && common_dir(&path)? == plan.common_dir
                && git(&path, &["symbolic-ref", "HEAD"])? == reference,
            "Existing worktree identity differs from plan"
        );
        if !plan.ready {
            ensure!(
                git(&path, &["rev-parse", "HEAD"])? == plan.commit
                    && git(&path, &["status", "--porcelain", "--untracked-files=all"])?.is_empty(),
                "Interrupted workspace creation has changes; inspect manually before retrying"
            );
        }
    } else {
        ensure!(
            !plan.ready,
            "Previously prepared workspace is missing; restore it before retrying"
        );
        let refs = git(
            &repository,
            &["for-each-ref", "--format=%(objectname)", &reference],
        )?;
        if refs.is_empty() {
            git(
                &repository,
                &[
                    "update-ref",
                    &reference,
                    &plan.commit,
                    &"0".repeat(plan.commit.len()),
                ],
            )?;
        } else {
            ensure!(
                refs == plan.commit,
                "Planned branch changed; refusing to reset it"
            );
        }
        git(
            &repository,
            &[
                "worktree",
                "add",
                "--",
                path.to_str().context("Non-UTF8 workspace path")?,
                &spec.branch,
            ],
        )?;
    }
    plan.ready = true;
    save(journal, key(command), &plan)?;
    Ok(Workspace {
        path: path.to_str().context("Non-UTF8 workspace path")?.into(),
        branch: spec.branch.clone(),
        base_commit: plan.commit,
    })
}

pub fn existing(
    journal: &Journal,
    command: &RunnerCommand,
    mapping: Option<&Path>,
    root: &Path,
) -> Result<Workspace> {
    let data: Option<String> = journal
        .db
        .query_row(
            "SELECT data FROM workspaces WHERE stream_id=?1",
            [key(command)],
            |r| r.get(0),
        )
        .optional()?;
    let plan: Plan =
        serde_json::from_str(&data.context("Prepare workspace on this runner first")?)?;
    ensure!(plan.ready, "Workspace preparation is not confirmed");
    prepare_inner(journal, command, mapping, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        _dir: tempfile::TempDir,
        repository: PathBuf,
        mapping: PathBuf,
        root: PathBuf,
        journal_path: PathBuf,
        journal: Journal,
        project: ProjectId,
        runner: RunnerId,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let repository = dir.path().join("source");
            fs::create_dir(&repository).unwrap();
            git(&repository, &["init", "-b", "main"]).unwrap();
            git(&repository, &["config", "user.name", "Factory Test"]).unwrap();
            git(
                &repository,
                &["config", "user.email", "factory@example.invalid"],
            )
            .unwrap();
            fs::write(repository.join("file.txt"), "initial\n").unwrap();
            git(&repository, &["add", "file.txt"]).unwrap();
            git(&repository, &["commit", "-m", "initial"]).unwrap();
            let project = ProjectId::new();
            let runner = RunnerId::new();
            let mapping = dir.path().join("repositories.json");
            fs::write(
                &mapping,
                serde_json::to_vec(&BTreeMap::from([(project, repository.clone())])).unwrap(),
            )
            .unwrap();
            let journal_path = dir.path().join("journal.db");
            let journal = Journal::open(&journal_path, "http://localhost:7432", runner).unwrap();
            let root = dir.path().join("workspaces");
            Self {
                _dir: dir,
                repository,
                mapping,
                root,
                journal_path,
                journal,
                project,
                runner,
            }
        }
        fn command(&self) -> RunnerCommand {
            let stream_id = StreamId::new();
            RunnerCommand {
                id: CommandId::new(),
                stream_id,
                runner_id: self.runner,
                ownership_epoch: 1,
                kind: CommandKind::PrepareWorkspace,
                agent_spec: None,
                agent_progress: None,
                state: CommandState::Accepted,
                result: None,
                created_unix_ms: 1,
                workspace_spec: Some(WorkspaceSpec {
                    checkpoint: None,
                    project_id: self.project,
                    branch: format!("factory/{stream_id}"),
                    base_branch: "main".into(),
                }),
            }
        }
        fn prepare(&self, command: &RunnerCommand) -> Result<Workspace> {
            prepare_inner(&self.journal, command, Some(&self.mapping), &self.root)
        }
        fn plan(&self, command: &RunnerCommand) -> Plan {
            let data: String = self
                .journal
                .db
                .query_row(
                    "SELECT data FROM workspaces WHERE stream_id=?1",
                    [command.stream_id.to_string()],
                    |r| r.get(0),
                )
                .unwrap();
            serde_json::from_str(&data).unwrap()
        }
    }
    #[test]
    fn two_streams_are_isolated_and_replay_preserves_dirty_source_and_worktrees() {
        let fixture = Fixture::new();
        fs::write(fixture.repository.join("file.txt"), "user source changes\n").unwrap();
        let a = fixture.command();
        let b = fixture.command();
        let first = fixture.prepare(&a).unwrap();
        let second = fixture.prepare(&b).unwrap();
        fs::write(Path::new(&first.path).join("file.txt"), "stream A\n").unwrap();
        fs::write(Path::new(&second.path).join("file.txt"), "stream B\n").unwrap();
        assert_eq!(fixture.prepare(&a).unwrap(), first);
        assert_eq!(
            fs::read_to_string(fixture.repository.join("file.txt")).unwrap(),
            "user source changes\n"
        );
        assert_eq!(
            fs::read_to_string(Path::new(&first.path).join("file.txt")).unwrap(),
            "stream A\n"
        );
        assert_eq!(
            fs::read_to_string(Path::new(&second.path).join("file.txt")).unwrap(),
            "stream B\n"
        );
    }
    #[test]
    fn occupied_path_and_unjournaled_branch_are_never_overwritten() {
        let fixture = Fixture::new();
        let command = fixture.command();
        let path = fixture.root.join(command.stream_id.to_string());
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("keep"), "user data").unwrap();
        assert!(fixture.prepare(&command).is_err());
        assert_eq!(fs::read_to_string(path.join("keep")).unwrap(), "user data");
        let other = fixture.command();
        git(
            &fixture.repository,
            &["branch", &other.workspace_spec.as_ref().unwrap().branch],
        )
        .unwrap();
        assert!(fixture.prepare(&other).is_err());
        assert!(!fixture.root.join(other.stream_id.to_string()).exists());
    }
    #[test]
    fn restart_reconciles_completed_git_operation_but_refuses_ambiguous_dirty_checkout() {
        let mut fixture = Fixture::new();
        let command = fixture.command();
        let workspace = fixture.prepare(&command).unwrap();
        let mut plan = fixture.plan(&command);
        plan.ready = false;
        save(&fixture.journal, command.stream_id, &plan).unwrap();
        drop(fixture.journal);
        fixture.journal = Journal::open(
            &fixture.journal_path,
            "http://localhost:7432",
            fixture.runner,
        )
        .unwrap();
        assert_eq!(fixture.prepare(&command).unwrap(), workspace);
        save(&fixture.journal, command.stream_id, &plan).unwrap();
        fs::write(
            Path::new(&workspace.path).join("file.txt"),
            "ambiguous edits",
        )
        .unwrap();
        assert!(fixture.prepare(&command).is_err());
        assert_eq!(
            fs::read_to_string(Path::new(&workspace.path).join("file.txt")).unwrap(),
            "ambiguous edits"
        );
    }
    #[test]
    fn branch_only_interruption_uses_pinned_commit_and_missing_ready_worktree_is_not_recreated() {
        let fixture = Fixture::new();
        let command = fixture.command();
        let workspace = fixture.prepare(&command).unwrap();
        let mut plan = fixture.plan(&command);
        git(
            &fixture.repository,
            &["worktree", "remove", &workspace.path],
        )
        .unwrap();
        assert!(fixture.prepare(&command).is_err());
        plan.ready = false;
        save(&fixture.journal, command.stream_id, &plan).unwrap();
        fs::write(fixture.repository.join("file.txt"), "new main commit").unwrap();
        git(&fixture.repository, &["commit", "-am", "advance main"]).unwrap();
        assert_eq!(fixture.prepare(&command).unwrap(), workspace);
        assert_eq!(
            fs::read_to_string(Path::new(&workspace.path).join("file.txt")).unwrap(),
            "initial\n"
        );
    }
}
