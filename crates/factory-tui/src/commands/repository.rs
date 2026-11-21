use anyhow::{bail, ensure, Context, Result};
use clap::Subcommand;
use factory_core::ProjectId;
use factory_protocol::client::FactoryClient;
#[cfg(any(unix, test))]
use std::fs::File;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Subcommand)]
pub enum RepositoryCommand {
    /// Clone the project's configured source into a NEW local directory and register it.
    Clone {
        #[arg(long)]
        project: ProjectId,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        repositories: PathBuf,
    },
    /// Register an existing local checkout without modifying it.
    Register {
        #[arg(long)]
        project: ProjectId,
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        repositories: PathBuf,
    },
    /// Fetch origin into remote-tracking refs only; never checkout, merge or reset.
    Fetch {
        #[arg(long)]
        project: ProjectId,
        #[arg(long)]
        repositories: PathBuf,
    },
}

pub async fn execute(client: &FactoryClient, command: RepositoryCommand) -> Result<()> {
    let id = match &command {
        RepositoryCommand::Clone { project, .. }
        | RepositoryCommand::Register { project, .. }
        | RepositoryCommand::Fetch { project, .. } => *project,
    };
    let project = client
        .snapshot()
        .await?
        .projects
        .into_iter()
        .find(|p| p.id == id)
        .context("Project not found")?;
    let path = tokio::task::spawn_blocking(move || {
        setup(command, &project.repository, &project.base_branch)
    })
    .await??;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({"project_id":id,"directory":path}))?
    );
    Ok(())
}

fn git(directory: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    command.env("GIT_TERMINAL_PROMPT", "0").args(args);
    if let Some(path) = directory {
        command.current_dir(path);
    }
    let output = command.output().context("Cannot start git")?;
    // A credential helper can include secrets in stderr. Report only the exit code.
    ensure!(
        output.status.success(),
        "Git operation failed ({}); check repository access and local Git authentication",
        output.status
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn checkout(path: &Path, branch: &str) -> Result<PathBuf> {
    let path = path.canonicalize()?;
    let root =
        PathBuf::from(git(Some(&path), &["rev-parse", "--show-toplevel"])?).canonicalize()?;
    ensure!(
        root == path,
        "Select the repository root, not a subdirectory"
    );
    git(
        Some(&path),
        &[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{branch}^{{commit}}"),
        ],
    )?;
    Ok(path)
}

fn setup(command: RepositoryCommand, source: &str, branch: &str) -> Result<PathBuf> {
    let (id, mapping) = match &command {
        RepositoryCommand::Clone {
            project,
            repositories,
            ..
        }
        | RepositoryCommand::Register {
            project,
            repositories,
            ..
        }
        | RepositoryCommand::Fetch {
            project,
            repositories,
        } => (*project, repositories),
    };
    let mapping = std::path::absolute(mapping)?;
    ensure!(
        !mapping.is_symlink(),
        "Repository mapping must not be a symlink"
    );
    let lock_path = mapping.with_file_name(format!(
        "{}.lock",
        mapping
            .file_name()
            .context("Missing mapping filename")?
            .to_string_lossy()
    ));
    let lock = super::private_options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    lock.try_lock().context("Repository mapping is busy")?;
    let before = match fs::read(&mapping) {
        Ok(value) => Some(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let mut entries: BTreeMap<ProjectId, PathBuf> = before
        .as_ref()
        .map(|v| serde_json::from_slice(v))
        .transpose()?
        .unwrap_or_default();
    let path = match command {
        RepositoryCommand::Clone { destination, .. } => {
            ensure!(
                !entries.contains_key(&id),
                "Project already mapped; use fetch or register the same path"
            );
            ensure!(
                !source.starts_with('-') && !source.is_empty(),
                "Invalid repository source"
            );
            git(None, &["check-ref-format", "--branch", branch])?;
            let destination = std::path::absolute(destination)?;
            // Atomic reservation; even an empty pre-existing directory is never reused.
            fs::create_dir(&destination)
                .context("Clone destination must be new and its parent must exist")?;
            git(
                None,
                &[
                    "clone",
                    "--origin",
                    "origin",
                    "--branch",
                    branch,
                    "--",
                    source,
                    destination.to_str().context("Non-UTF8 destination")?,
                ],
            )
            .context("Clone incomplete; directory retained for inspection, mapping not changed")?;
            checkout(&destination, branch)?
        }
        RepositoryCommand::Register { directory, .. } => checkout(&directory, branch)?,
        RepositoryCommand::Fetch { .. } => {
            let path = entries
                .get(&id)
                .context("Project has no local repository mapping")?;
            let path = checkout(path, branch)?;
            // Explicit destination refspec ignores configured fetch mappings into local branches.
            // Do not import tags, prune, recurse into submodules or change FETCH_HEAD.
            git(
                Some(&path),
                &[
                    "fetch",
                    "--no-prune",
                    "--no-prune-tags",
                    "--no-tags",
                    "--no-recurse-submodules",
                    "--no-write-fetch-head",
                    "origin",
                    "+refs/heads/*:refs/remotes/origin/*",
                ],
            )?;
            return Ok(path);
        }
    };
    if let Some(existing) = entries.get(&id) {
        ensure!(
            existing.canonicalize()? == path,
            "Project already mapped to another repository; refusing to retarget it"
        );
        return Ok(path);
    }
    entries.insert(id, path.clone());
    // Detect non-cooperating edits too; cooperating commands hold the sibling lock.
    let current = match fs::read(&mapping) {
        Ok(value) => Some(value),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if before != current {
        bail!("Mapping changed during setup; repository retained, retry register");
    }
    let temp = mapping.with_file_name(format!(
        ".repositories-{}.tmp",
        factory_core::RequestId::new()
    ));
    let result = (|| -> Result<()> {
        let mut file = super::private_options()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(&entries)?)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temp, &mapping)?;
        #[cfg(unix)]
        File::open(mapping.parent().context("Missing mapping parent")?)?.sync_all()?;
        Ok(())
    })();
    if temp.exists() {
        let _ = fs::remove_file(temp);
    }
    result?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn occupied_mapping_lock_and_failed_clone_leave_existing_entries_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let mapping = tmp.path().join("repos.json");
        let original =
            serde_json::to_vec(&serde_json::json!({ProjectId::new().to_string():"/existing"}))
                .unwrap();
        fs::write(&mapping, &original).unwrap();
        let lock = File::create(tmp.path().join("repos.json.lock")).unwrap();
        lock.lock().unwrap();
        let dest = tmp.path().join("clone");
        assert!(setup(
            RepositoryCommand::Clone {
                project: ProjectId::new(),
                destination: dest.clone(),
                repositories: mapping.clone()
            },
            "/missing-source",
            "main"
        )
        .is_err());
        assert!(!dest.exists());
        drop(lock);
        assert!(setup(
            RepositoryCommand::Clone {
                project: ProjectId::new(),
                destination: dest,
                repositories: mapping.clone()
            },
            tmp.path().join("missing").to_str().unwrap(),
            "main"
        )
        .is_err());
        assert_eq!(fs::read(mapping).unwrap(), original);
    }
    #[test]
    fn clone_fetch_preserves_dirty_checkout_and_mapping_never_retargets() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        fs::create_dir(&source).unwrap();
        git(Some(&source), &["init", "-b", "main"]).unwrap();
        git(
            Some(&source),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ],
        )
        .unwrap();
        let id = ProjectId::new();
        let mapping = tmp.path().join("repos.json");
        let dest = tmp.path().join("checkout");
        setup(
            RepositoryCommand::Clone {
                project: id,
                destination: dest.clone(),
                repositories: mapping.clone(),
            },
            source.to_str().unwrap(),
            "main",
        )
        .unwrap();
        fs::write(dest.join("dirty"), "keep me").unwrap();
        let head = git(Some(&dest), &["rev-parse", "HEAD"]).unwrap();
        git(
            Some(&source),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "next",
            ],
        )
        .unwrap();
        setup(
            RepositoryCommand::Fetch {
                project: id,
                repositories: mapping.clone(),
            },
            "unused",
            "main",
        )
        .unwrap();
        assert_eq!(git(Some(&dest), &["rev-parse", "HEAD"]).unwrap(), head);
        assert_ne!(
            git(Some(&dest), &["rev-parse", "refs/remotes/origin/main"]).unwrap(),
            head
        );
        assert_eq!(fs::read_to_string(dest.join("dirty")).unwrap(), "keep me");
        let before = fs::read(&mapping).unwrap();
        assert!(setup(
            RepositoryCommand::Register {
                project: id,
                directory: source.clone(),
                repositories: mapping.clone()
            },
            "unused",
            "main"
        )
        .is_err());
        assert_eq!(fs::read(&mapping).unwrap(), before);
        assert!(setup(
            RepositoryCommand::Clone {
                project: ProjectId::new(),
                destination: dest.clone(),
                repositories: mapping.clone()
            },
            source.to_str().unwrap(),
            "main"
        )
        .is_err());
        setup(
            RepositoryCommand::Register {
                project: id,
                directory: dest,
                repositories: mapping,
            },
            "unused",
            "main",
        )
        .unwrap();
    }
}
