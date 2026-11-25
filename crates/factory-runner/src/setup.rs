use crate::{journal::Journal, workspace, Args};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn effective(args: &Args, c: &RunnerCommand) -> Result<Args> {
    let mut args = args.clone();
    if c.kind == CommandKind::TargetGit {
        return Ok(args);
    }
    if let Some(s) = c.workspace_spec.as_ref().and_then(|s| s.setup.as_ref()) {
        ensure!(
            s.runner_id == c.runner_id,
            "Project setup belongs to another runner"
        );
        if !s.service_registration.is_empty() {
            ensure!(args.managed_opencode.is_none(), "Externe OpenCode-Registrierung ist auf diesem verwalteten Runner nicht erlaubt");
            args.opencode_registration = Some(PathBuf::from(&s.service_registration));
        }
    }
    Ok(args)
}
fn harness(path: &Path, value: &serde_json::Value) -> Result<()> {
    let target = path.join("opencode.json");
    // A generated harness must never silently merge over an existing project harness.
    for other in [
        "opencode.jsonc",
        ".opencode/opencode.json",
        ".opencode/opencode.jsonc",
    ] {
        ensure!(fs::symlink_metadata(path.join(other)).is_err(),"Repository already has OpenCode config; select repository configuration in project setup");
    }
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    if fs::symlink_metadata(&target).is_ok() {
        ensure!(!fs::symlink_metadata(&target)?.file_type().is_symlink() && fs::read(&target)?==bytes,"Existing opencode.json differs; select repository configuration instead of replacing it");
        return Ok(());
    }
    let mut temp = tempfile::NamedTempFile::new_in(path)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(&target)
        .map_err(|_| anyhow::anyhow!("Harness path occupied during setup"))?;
    factory_platform::sync_dir(path)?;
    Ok(())
}
pub async fn prepare(
    j: &Journal,
    record: &mut crate::journal::Record,
    args: &Args,
) -> CommandOutput {
    if let Err(e) = crate::managed::ensure_repository(j, record, args).await {
        return CommandOutput::WorkspaceError {
            workspace_error: e.to_string().chars().take(240).collect(),
        };
    }
    let c = &record.command;
    let output = tokio::task::block_in_place(|| {
        workspace::prepare(
            j,
            c,
            args.repositories.as_deref(),
            args.workspace_root
                .as_deref()
                .expect("workspace root initialized"),
        )
    });
    let CommandOutput::Workspace { workspace } = &output else {
        return output;
    };
    if args.managed_opencode.is_some() {
        if let Err(e) = tokio::task::block_in_place(|| crate::managed_opencode::validate_workspace(Path::new(&workspace.path))) {
            return CommandOutput::WorkspaceError { workspace_error: e.to_string().chars().take(240).collect() };
        }
    }
    let Some(s) = c.workspace_spec.as_ref().and_then(|s| s.setup.as_ref()) else {
        return output;
    };
    let checked = async {
        if let Some(config) = &s.harness {
            tokio::task::block_in_place(|| {
                let path = Path::new(&workspace.path);
                if args.managed_opencode.is_some() {
                    crate::managed_opencode::validate_config(Path::new("Factory-Harness"), config)?;
                }
                harness(path, config)
            })?;
        } else {
            for relative in ["opencode.json","opencode.jsonc",".opencode"] {
                if !s.repository_path.is_empty() && fs::symlink_metadata(Path::new(&s.repository_path).join(relative)).is_ok() {
                    ensure!(fs::symlink_metadata(Path::new(&workspace.path).join(relative)).is_ok(),"Project harness is absent from the checkout; commit its config or provide harness JSON in the wizard");
                }
            }
        }
        let path = args
            .opencode_registration
            .as_ref()
            .context("OpenCode registration missing")?;
        let status = async {
            match factory_opencode::OpenCodeClient::from_registration(path) {
                Ok(client) => client.status().await,
                Err(_) => OpenCodeStatus::Unavailable,
            }
        }.await;
        if let OpenCodeStatus::Unsupported { version } = &status {
            anyhow::bail!("OpenCode {version} wird nicht unterstützt; benötigt wird OpenCode V2 ab 2.0.27. Der Dienst wurde nicht verändert.");
        }
        ensure!(
            matches!(status, OpenCodeStatus::Ready { .. }),
            "OpenCode nicht erreichbar oder inkompatibel; benötigt wird OpenCode V2 ab 2.0.27"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    match checked {
        Ok(()) => output,
        Err(e) => CommandOutput::WorkspaceError {
            workspace_error: e.to_string().chars().take(240).collect(),
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn harness_replay_never_overwrites_existing_configuration_or_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let config = serde_json::json!({"mcp":{"servers":{}},"instructions":["RULES.md"]});
        harness(dir.path(), &config).unwrap();
        harness(dir.path(), &config).unwrap();
        fs::write(dir.path().join("opencode.json"), "user changes").unwrap();
        assert!(harness(dir.path(), &config).is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("opencode.json")).unwrap(),
            "user changes"
        );
        let second = tempfile::tempdir().unwrap();
        fs::write(second.path().join("opencode.jsonc"), "{}").unwrap();
        assert!(harness(second.path(), &config).is_err());
        let third = tempfile::tempdir().unwrap();
        crate::test_symlink_file(
            dir.path().join("opencode.json"),
            third.path().join("opencode.json"),
        )
        .unwrap();
        assert!(harness(third.path(), &config).is_err());
    }
}
