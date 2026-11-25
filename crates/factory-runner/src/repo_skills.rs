use crate::workspace::git;
use anyhow::{ensure, Context, Result};
use factory_core::{valid_repo_skill_id, Workspace};
use std::{fs, path::Path};

/// Compare every native skill file with the immutable commit chosen at preparation.
/// The OpenCode location stays the original worktree; no generated files enter Git.
pub(crate) fn verify(workspace: &Workspace, ids: &[String]) -> Result<()> {
    let root = Path::new(&workspace.path);
    ensure!(
        matches!(workspace.base_commit.len(), 40 | 64)
            && workspace.base_commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "Ungültiger Basiscommit für Repo-Skills"
    );
    ensure!(ids.len() <= 32, "Zu viele Repo-Skills");
    let mut file_count = 0;
    for id in ids {
        ensure!(valid_repo_skill_id(id), "Ungültige Skill-ID");
        let relative = format!(".opencode/skills/{id}");
        let tree = git(
            root,
            &["ls-tree", "-rz", &workspace.base_commit, "--", &relative],
        )?;
        let records = tree.split('\0').filter(|s| !s.is_empty());
        let mut files = std::collections::BTreeMap::new();
        let prefix = format!("{relative}/");
        ensure!(
            fs::symlink_metadata(root.join(".opencode/skills"))?.is_dir(),
            "Repo-Skill-Quelle ist kein regulärer Ordner"
        );
        for record in records {
            let (fields, path) = record
                .split_once('\t')
                .context("Ungültiger Git-Baumeintrag")?;
            let mut fields = fields.split_ascii_whitespace();
            let mode = fields.next();
            ensure!(
                matches!(mode, Some("100644" | "100755")) && fields.next() == Some("blob"),
                "Repo-Skill {id} enthält einen Symlink oder ein Submodul"
            );
            let digest = fields.next().context("Git-Objekt fehlt")?;
            ensure!(
                path.starts_with(&prefix)
                    && files.insert(path, digest).is_none()
                    && files.len() <= 128,
                "Repo-Skill {id} hat zu viele oder ungültige Dateien"
            );
            file_count += 1;
            ensure!(
                file_count <= 256,
                "Zu viele Dateien in freigegebenen Repo-Skills"
            );
        }
        ensure!(
            files.contains_key(format!("{relative}/SKILL.md").as_str()),
            "Repo-Skill {id} fehlt im vorbereiteten Git-Stand"
        );
        inspect(root, &relative, &files, 0)?;
        for (path, digest) in files {
            let actual = fs::symlink_metadata(root.join(path))
                .with_context(|| format!("Datei für Repo-Skill {id} fehlt im Worktree"))?;
            ensure!(
                actual.is_file() && actual.len() <= 1024 * 1024,
                "Repo-Skill {id} enthält keine reguläre Datei oder überschreitet das Dateilimit"
            );
            ensure!(
                git(root, &["hash-object", "--no-filters", "--", path])? == digest,
                "Repo-Skill {id} weicht vom vorbereiteten Git-Stand ab"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_commit_and_rejects_changed_untracked_and_symlinked_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let skill = root.join(".opencode/skills/review");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "# Review").unwrap();
        fs::write(skill.join("reference.md"), "original").unwrap();
        git(root, &["init"]).unwrap();
        git(root, &["add", "."]).unwrap();
        git(
            root,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-m",
                "Prepare",
            ],
        )
        .unwrap();
        let workspace = Workspace {
            path: root.to_str().unwrap().into(),
            branch: "test".into(),
            base_commit: git(root, &["rev-parse", "HEAD"]).unwrap(),
        };
        let ids = vec!["review".into()];
        verify(&workspace, &ids).unwrap();
        fs::write(skill.join("reference.md"), "changed").unwrap();
        assert!(verify(&workspace, &ids)
            .unwrap_err()
            .to_string()
            .contains("Git-Stand"));
        fs::write(skill.join("reference.md"), "original").unwrap();
        fs::write(skill.join("extra.md"), "extra").unwrap();
        assert!(verify(&workspace, &ids)
            .unwrap_err()
            .to_string()
            .contains("nicht eingecheckte"));
        fs::remove_file(skill.join("extra.md")).unwrap();
        fs::remove_file(skill.join("reference.md")).unwrap();
        crate::test_symlink_file(skill.join("SKILL.md"), skill.join("reference.md")).unwrap();
        assert!(verify(&workspace, &ids).is_err());
    }
}

fn inspect(
    root: &Path,
    relative: &str,
    expected: &std::collections::BTreeMap<&str, &str>,
    depth: u8,
) -> Result<()> {
    ensure!(depth <= 12, "Repo-Skill-Verzeichnis ist zu tief");
    ensure!(
        fs::symlink_metadata(root.join(relative))?.is_dir(),
        "Repo-Skill-Verzeichnis ist kein regulärer Ordner"
    );
    for entry in fs::read_dir(root.join(relative))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .context("Ungültiger Dateiname im Repo-Skill")?;
        let relative = format!("{relative}/{name}");
        let kind = entry.file_type()?;
        if kind.is_dir() {
            inspect(root, &relative, expected, depth + 1)?;
        } else {
            ensure!(
                kind.is_file() && expected.contains_key(relative.as_str()),
                "Repo-Skill enthält nicht eingecheckte Dateien"
            );
        }
    }
    Ok(())
}
