//! Local Git worktrees with a durable pre-operation plan. Preparation never resets, cleans or deletes.
use crate::journal::Journal;
use anyhow::{bail, ensure, Context, Result};
use factory_core::*;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
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

/// Keep canonical paths for filesystem checks and journal bindings, but pass
/// Windows drive and UNC paths to Git without the Win32 verbatim prefix.
pub(crate) fn git_path(path: &Path) -> Result<Cow<'_, str>> {
    let text = path.to_str().context("Non-UTF8 Git path")?;
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if let Some(Component::Prefix(prefix)) = path.components().next() {
            match prefix.kind() {
                Prefix::VerbatimDisk(_) => return Ok(text[4..].replace('\\', "/").into()),
                Prefix::VerbatimUNC(_, _) => {
                    return Ok(format!("//{}", text[8..].replace('\\', "/")).into());
                }
                Prefix::Verbatim(_) | Prefix::DeviceNS(_) => return Ok(Cow::Borrowed(text)),
                _ => {}
            }
        }
        if text.contains('\\') {
            return Ok(text.replace('\\', "/").into());
        }
    }
    Ok(Cow::Borrowed(text))
}

// Arguments are passed directly, never through a shell. Output goes to local
// temp files to avoid pipe deadlocks while enforcing a command timeout.
pub(crate) fn git(repository: &Path, args: &[&str]) -> Result<String> {
    git_env(repository, args, &[])
}
pub(crate) fn git_env(repository: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
    // Git for Windows can take more than five seconds for fixture operations
    // when tests are running in parallel on hosted CI. Keep production bounded
    // by the existing timeout.
    let timeout = if cfg!(all(test, windows)) { 30 } else { 5 };
    git_with_timeout(repository, args, env, Duration::from_secs(timeout))
}
pub(crate) fn git_network(repository: &Path, args: &[&str]) -> Result<String> {
    git_with_timeout(repository, args, &[], Duration::from_secs(60))
}
fn git_with_timeout(
    repository: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    timeout: Duration,
) -> Result<String> {
    let (_, output) = git_status(repository, args, env, timeout, false)?;
    Ok(output)
}

/// An ordinary merge-tree conflict exits 1 and carries useful file diagnostics.
pub(crate) fn merge_tree(
    repository: &Path,
    base: &str,
    target: &str,
    source: &str,
) -> Result<(bool, String)> {
    // A repository's custom merge drivers are arbitrary programs. Automatic
    // publication must never execute them; report those merges as conflicts.
    let config = git(repository, &["config", "--list", "--name-only"])?;
    let mut args = vec![
        "-c".to_string(),
        "merge.renames=false".to_string(),
        "-c".to_string(),
        "merge.renormalize=false".to_string(),
    ];
    for key in config
        .lines()
        .filter(|k| k.starts_with("merge.") && k.ends_with(".driver"))
    {
        args.extend(["-c".into(), format!("{key}=false")]);
    }
    args.extend(
        [
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--merge-base",
            base,
            target,
            source,
        ]
        .into_iter()
        .map(str::to_owned),
    );
    git_status(
        repository,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        &[],
        Duration::from_secs(30),
        true,
    )
}
fn git_status(
    repository: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    timeout: Duration,
    allow_conflict: bool,
) -> Result<(bool, String)> {
    let out = tempfile::tempfile()?;
    #[cfg(test)]
    let mut err = tempfile::tempfile()?;
    let mut command = Command::new("git");
    // Git for Windows rejects a long explicit GIT_DIR before core.longpaths
    // applies, and passes that value to helpers such as pack-objects. Address
    // bare repositories by their short name relative to the direct parent.
    // This also avoids using the repository itself as CreateProcess.current_dir.
    // Do not modify the user's Git config.
    #[cfg(windows)]
    let bare_init = args.first() == Some(&"init") && args.contains(&"--bare");
    #[cfg(windows)]
    {
        command.args(["-c", "core.longpaths=true"]);
        if !bare_init
            && !repository.join(".git").exists()
            && repository.join("HEAD").is_file()
            && repository.join("objects").is_dir()
        {
            let parent = repository
                .parent()
                .context("Bare repository parent missing")?;
            let name = repository
                .file_name()
                .context("Bare repository name missing")?;
            command
                .current_dir(parent)
                .arg("--git-dir")
                .arg(name);
        } else if !bare_init {
            command.arg("-C").arg(git_path(repository)?.as_ref());
        }
    }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    // Hosted CI images install global filters such as Git LFS. Test fixtures
    // must not inherit machine-specific Git behavior, especially when testing
    // that checkpoint returns reject configured checkout filters.
    #[cfg(test)]
    command
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env(
            "GIT_CONFIG_SYSTEM",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        );
    #[cfg(not(windows))]
    command.current_dir(repository);
    command
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(out.try_clone()?);
    #[cfg(test)]
    command.stderr(err.try_clone()?);
    #[cfg(not(test))]
    command.stderr(Stdio::null());
    // `git init <path>` loads command configuration before creating the target,
    // unlike both CreateProcess.current_dir and Git's startup-level -C option.
    #[cfg(windows)]
    if bare_init {
        command.arg(git_path(repository)?.as_ref());
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;
    let success = loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() && !(allow_conflict && status.code() == Some(1)) {
                #[cfg(test)]
                {
                    use std::io::{Read, Seek, SeekFrom};
                    err.seek(SeekFrom::Start(0))?;
                    let mut diagnostic = String::new();
                    err.take(8_193).read_to_string(&mut diagnostic)?;
                    ensure!(
                        diagnostic.len() <= 8_192,
                        "Git test diagnostic exceeds inspection limit"
                    );
                    bail!("Git fixture operation failed: {}", diagnostic.trim());
                }
                #[cfg(not(test))]
                bail!("Git operation failed; inspect repository/worktree before retrying");
            }
            break status.success();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Git operation timed out; inspect repository/worktree before retrying");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    use std::io::{Read, Seek, SeekFrom};
    let mut out = out;
    out.seek(SeekFrom::Start(0))?;
    let mut text = String::new();
    out.take(1_048_577).read_to_string(&mut text)?;
    ensure!(
        text.len() <= 1_048_576,
        "Git output exceeds inspection limit"
    );
    Ok((success, if args.contains(&"-z") { text } else { text.trim().to_string() }))
}

pub(crate) fn common_dir(repository: &Path) -> Result<PathBuf> {
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
fn path_key(command: &RunnerCommand) -> String {
    let stream = command.stream_id.to_string().replace('-', "");
    if command.ownership_epoch == 1 {
        stream
    } else {
        format!("{stream}e{}", command.ownership_epoch)
    }
}

pub(crate) fn has_plan(journal: &Journal, command: &RunnerCommand) -> Result<bool> {
    Ok(journal.db.query_row("SELECT EXISTS(SELECT 1 FROM workspaces WHERE stream_id=?1)",
        [key(command)], |row| row.get(0))?)
}

/// Only an explicit draft reset reaches here. Bind the removal to the recorded
/// linked worktree, never the user's checkout, a reused path or a symlink.
pub(crate) fn discard(
    journal: &Journal, command: &RunnerCommand, workspace: &Workspace,
    root: &Path, verified: bool, remove: bool,
) -> Result<()> {
    let raw: String = journal.db.query_row("SELECT data FROM workspaces WHERE stream_id=?1",
        [key(command)], |row| row.get(0)).context("No journaled worktree")?;
    let plan: Plan = serde_json::from_str(&raw)?;
    let root = fs::canonicalize(root)?;
    ensure!(plan.ready && plan.path.parent() == Some(root.as_path())
        && (plan.path == root.join(path_key(command)) || plan.path == root.join(key(command)))
        && git_path(&plan.path)? == git_path(Path::new(&workspace.path))?
        && plan.spec.branch == workspace.branch && plan.path != plan.repository,
        "Draft reset worktree binding changed");
    ensure!(fs::canonicalize(&plan.repository)? == plan.repository
        && common_dir(&plan.repository)? == plan.common_dir, "Draft reset repository changed");
    let path = git_path(&plan.path)?;
    let listed = || -> Result<bool> {
        let output = git(&plan.repository, &["worktree", "list", "--porcelain", "-z"])?;
        Ok(output.split('\0').any(|line| line.strip_prefix("worktree ") == Some(path.as_ref())))
    };
    match fs::symlink_metadata(&plan.path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ensure!(verified && !listed()?, "No verified removal of the worktree");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
        Ok(meta) => ensure!(meta.file_type().is_dir(), "Draft reset path is not an ordinary directory"),
    }
    ensure!(fs::canonicalize(&plan.path)? == plan.path
        && fs::symlink_metadata(plan.path.join(".git"))?.file_type().is_file()
        && common_dir(&plan.path)? == plan.common_dir
        && git(&plan.path, &["symbolic-ref", "HEAD"])? == format!("refs/heads/{}", workspace.branch)
        && listed()?, "Draft reset worktree identity changed");
    if remove {
        ensure!(verified, "Missing local draft reset confirmation");
        git(&plan.repository, &["worktree", "remove", "--force", "--", &path])?;
        ensure!(!listed()? && matches!(fs::symlink_metadata(&plan.path), Err(e) if e.kind() == std::io::ErrorKind::NotFound),
            "Worktree removal not confirmed");
    }
    Ok(())
}

pub fn prepare(
    journal: &Journal,
    command: &RunnerCommand,
    mapping: Option<&Path>,
    root: &Path,
) -> CommandOutput {
    match prepare_inner(journal, command, mapping, root, false) {
        Ok(workspace) => CommandOutput::Workspace { workspace },
        // Errors here are ours, never Git stderr (which could contain credentials).
        Err(error) => {
            #[cfg(test)]
            let message = format!("{error:#}");
            #[cfg(not(test))]
            let message = error.to_string();
            CommandOutput::WorkspaceError {
                workspace_error: message.chars().take(240).collect(),
            }
        }
    }
}

fn prepare_inner(
    journal: &Journal,
    command: &RunnerCommand,
    mapping: Option<&Path>,
    root: &Path,
    snapshot_only: bool,
) -> Result<Workspace> {
    let mut spec = command
        .workspace_spec
        .clone()
        .context("Workspace specification missing")?;
    // Post-publication Git belongs to this command, not the workspace binding.
    spec.target_git = None;
    let spec = &spec;
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
    let repository = repository(spec, mapping, root)?;
    let common = common_dir(&repository).context("Mapped path is not a Git repository")?;
    fs::create_dir_all(root).context("Cannot create workspace root")?;
    let root = fs::canonicalize(root)?;
    let previous: Option<String> = journal
        .db
        .query_row(
            "SELECT data FROM workspaces WHERE stream_id=?1",
            [key(command)],
            |r| r.get(0),
        )
        .optional()?;
    let previous: Option<Plan> = previous
        .map(|value| serde_json::from_str(&value))
        .transpose()?;
    if snapshot_only {
        ensure!(previous.as_ref().is_some_and(|plan| plan.ready), "Workspace preparation is not confirmed");
    }
    let new_path = root.join(path_key(command));
    let legacy_path = root.join(key(command));
    let path = previous
        .as_ref()
        .map(|plan| plan.path.clone())
        .unwrap_or_else(|| new_path.clone());
    let reference = format!("refs/heads/{}", spec.branch);
    let mut plan: Plan = if let Some(previous) = previous {
        ensure!(
            previous.spec == *spec
                && previous.repository == repository
                && previous.common_dir == common
                && (previous.path == new_path || previous.path == legacy_path),
            "Workspace mapping changed; restore original configuration before retrying"
        );
        previous
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
            crate::checkpoint::import_objects(&repository, &root, import)
                .context("Cannot import checkpoint into the workspace repository")?
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
            fs::symlink_metadata(path.join(".git")).is_ok_and(|meta| meta.file_type().is_file()),
            "Existing path is not the planned worktree; inspect manually"
        );
        ensure!(
            fs::canonicalize(git(&path, &["rev-parse", "--show-toplevel"])?)? == path
                && common_dir(&path)? == plan.common_dir,
            "Existing worktree identity differs from plan"
        );
        let head = git(&path, &["rev-parse", "--symbolic-full-name", "HEAD"])?;
        if snapshot_only && head == "HEAD" {
            let registered = git(&repository, &["worktree", "list", "--porcelain", "-z"])?;
            let path = git_path(&path)?;
            ensure!(registered.split('\0').any(|line| line.strip_prefix("worktree ") == Some(path.as_ref())),
                "Detached snapshot worktree is not registered in the planned repository");
            git(&plan.path, &["merge-base", "--is-ancestor", &plan.commit, "HEAD"])
                .context("Detached snapshot HEAD does not descend from the prepared commit")?;
        } else {
            ensure!(head == reference, "Existing worktree branch differs from plan");
        }
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
            )
            .context("Cannot create the planned workspace branch")?;
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
                &git_path(&path)?,
                &spec.branch,
            ],
        )
        .context("Cannot create the linked Git worktree")?;
    }
    if !snapshot_only {
        plan.ready = true;
        save(journal, key(command), &plan)?;
    }
    Ok(Workspace {
        path: path.to_str().context("Non-UTF8 workspace path")?.into(),
        branch: spec.branch.clone(),
        base_commit: plan.commit,
    })
}

pub(crate) fn repository(
    spec: &WorkspaceSpec,
    mapping: Option<&Path>,
    root: &Path,
) -> Result<PathBuf> {
    if let Some(setup) = &spec.setup {
        if setup.repository_path.is_empty() {
            return crate::managed::resolve(spec, root);
        }
        return fs::canonicalize(&setup.repository_path)
            .context("Project repository path does not exist on this runner");
    }
    let mapping =
        mapping.context("Configure project setup in the TUI or provide --repositories")?;
    let repositories: BTreeMap<ProjectId, PathBuf> =
        serde_json::from_slice(&fs::read(mapping).context("Cannot read repository mapping")?)?;
    fs::canonicalize(
        repositories
            .get(&spec.project_id)
            .context("Project has no local repository mapping")?,
    )
    .context("Mapped repository does not exist")
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
    prepare_inner(journal, command, mapping, root, false)
}

pub(crate) fn existing_for_snapshot(
    journal: &Journal, command: &RunnerCommand, mapping: Option<&Path>, root: &Path,
) -> Result<Workspace> {
    prepare_inner(journal, command, mapping, root, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn git_paths_use_drive_or_unc_syntax_without_verbatim_prefix() {
        for (path, expected) in [
            (r"\\?\C:\test folder\repo", "C:/test folder/repo"),
            (r"C:\test folder\repo", "C:/test folder/repo"),
            (r"\\?\UNC\server\share\repo", "//server/share/repo"),
            (r"\\server\share\repo", "//server/share/repo"),
        ] {
            assert_eq!(git_path(Path::new(path)).unwrap(), expected);
        }
    }

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
                    target_git: None,
                    setup: None,
                    checkpoint: None,
                    project_id: self.project,
                    branch: format!("factory/{stream_id}"),
                    base_branch: "main".into(),
                }),
            }
        }
        fn prepare(&self, command: &RunnerCommand) -> Result<Workspace> {
            prepare_inner(&self.journal, command, Some(&self.mapping), &self.root, false)
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
    fn draft_discard_removes_only_the_verified_linked_worktree_and_is_replayable() {
        let f = Fixture::new();
        let a = f.command();
        let b = f.command();
        let first = f.prepare(&a).unwrap();
        let second = f.prepare(&b).unwrap();
        fs::write(Path::new(&first.path).join("unpublished"), "discard me").unwrap();
        fs::write(Path::new(&second.path).join("unpublished"), "other intent").unwrap();
        fs::write(f.repository.join("file.txt"), "user change").unwrap();
        assert!(discard(&f.journal, &a, &second, &f.root, true, true).is_err());
        assert!(discard(&f.journal, &a, &first, &f.root, false, true).is_err());
        discard(&f.journal, &a, &first, &f.root, false, false).unwrap();
        discard(&f.journal, &a, &first, &f.root, true, true).unwrap();
        assert!(!Path::new(&first.path).exists());
        discard(&f.journal, &a, &first, &f.root, true, true).unwrap();
        assert_eq!(fs::read_to_string(Path::new(&second.path).join("unpublished")).unwrap(), "other intent");
        assert_eq!(fs::read_to_string(f.repository.join("file.txt")).unwrap(), "user change");
        fs::create_dir(&first.path).unwrap();
        fs::write(Path::new(&first.path).join("keep"), "replacement").unwrap();
        assert!(discard(&f.journal, &a, &first, &f.root, true, true).is_err());
        assert_eq!(fs::read_to_string(Path::new(&first.path).join("keep")).unwrap(), "replacement");
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
        let path = fixture.root.join(path_key(&command));
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
        assert!(!fixture.root.join(path_key(&other)).exists());
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
            &[
                "worktree",
                "remove",
                &git_path(Path::new(&workspace.path)).unwrap(),
            ],
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
