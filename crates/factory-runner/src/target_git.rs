//! Execute only the typed plan against the user-selected local project checkout.
use crate::{journal::{Journal, Record}, publication, workspace::{git, git_network}};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use std::{fs, path::Path};

fn version(tag: &str) -> Option<([u64; 3], bool)> {
    let bare = tag.strip_prefix('v').unwrap_or(tag);
    let mut fields = bare.split('.');
    let mut parts = [0; 3];
    for part in &mut parts {
        let field = fields.next()?;
        if !field.bytes().all(|b| b.is_ascii_digit()) || (field.len() > 1 && field.starts_with('0')) {
            return None;
        }
        *part = field.parse().ok()?;
    }
    fields.next().is_none().then_some((parts, bare != tag))
}

fn next_tag(tags: &str, bump: &TargetTag) -> Result<String> {
    let field = match bump {
        TargetTag::Named { name } => return Ok(name.clone()),
        TargetTag::Major => 0,
        TargetTag::Minor => 1,
        TargetTag::Patch => 2,
    };
    let Some((mut parts, prefix)) = tags.lines().filter_map(version).max() else {
        return Ok("v0.1.0".into());
    };
    parts[field] = parts[field].checked_add(1).context("Versionsnummer zu groß")?;
    parts[field + 1..].fill(0);
    let [major, minor, patch] = parts;
    Ok(format!("{}{major}.{minor}.{patch}", if prefix { "v" } else { "" }))
}

fn target_head(path: &Path, branch: &str) -> Result<String> {
    ensure!(git(path, &["symbolic-ref", "HEAD"])? == branch,
        "Zielbranch ist nicht ausgecheckt");
    git(path, &["rev-parse", "--verify", "HEAD"])
}

fn push_destination(path: &Path, spec: &WorkspaceSpec, plan: &TargetGitPlan, branch: &str) -> Result<(String, String)> {
    let (remote, dest) = match &plan.remote {
        Some(remote) => (remote.clone(), branch.to_string()),
        None => {
            let setting = |key| git(path, &["config", "--get-all", &format!("branch.{}.{key}", spec.base_branch)])
                .context("Kein eindeutiger Upstream für den Zielbranch konfiguriert");
            (setting("remote")?, setting("merge")?)
        }
    };
    ensure!(remote != "." && git(path, &["remote"])?.lines().any(|name| name == remote),
        "Kein eindeutiger konfigurierter Push-Remote");
    ensure!(dest.starts_with("refs/heads/"), "Upstream ist kein Branch");
    git(path, &["check-ref-format", &dest]).context("Ungültiger Push-Zielbranch")?;
    let urls = git(path, &["remote", "get-url", "--push", "--all", "--", &remote])?;
    ensure!(!urls.is_empty() && urls.lines().count() == 1, "Remote hat kein eindeutiges Push-Ziel");
    let mirror = git(path, &["config", "--type=bool", "--default=false", "--get", &format!("remote.{remote}.mirror")])?;
    ensure!(mirror == "false", "Mirror-Remotes sind für geplante Pushes nicht zulässig");
    Ok((remote, dest))
}

fn execute_inner(spec: &WorkspaceSpec, expected: Option<&str>, steps: &mut Vec<String>) -> Result<()> {
    let plan = spec.target_git.as_ref().filter(|p| p.valid()).context("Ungültiger Git-Plan")?;
    let setup = spec.setup.as_ref().filter(|s| s.has_local_target()).context("Lokaler Ziel-Checkout fehlt")?;
    let path = fs::canonicalize(&setup.repository_source).context("Projektordner fehlt")?;
    ensure!(fs::canonicalize(git(&path, &["rev-parse", "--show-toplevel"])?).ok().as_ref() == Some(&path),
        "Projektordner ist nicht die Wurzel eines Git-Checkouts");
    let branch = format!("refs/heads/{}", spec.base_branch);
    git(&path, &["check-ref-format", &branch])?;
    let mut head = target_head(&path, &branch)?;
    if let Some(expected) = expected {
        ensure!(head == expected, "Zielbranch wurde seit der Übernahme verändert");
        steps.push(format!("Übernahme: {head}"));
    }
    // Check the push destination and tag before making staging/ref changes.
    let push = plan.push.then(|| push_destination(&path, spec, plan, &branch)).transpose()?;
    let tag = if let Some(bump) = &plan.tag {
        let tags = git(&path, &["tag", "--merged", &head, "--list"])?;
        let tag = format!("refs/tags/{}", next_tag(&tags, bump)?);
        git(&path, &["check-ref-format", &tag]).context("Ungültiger Tag-Name")?;
        ensure!(!git(&path, &["for-each-ref", "--format=%(refname)", &tag])?.lines().any(|r| r == tag),
            "Tag existiert bereits und wird nicht überschrieben: {tag}");
        Some(tag)
    } else { None };
    if plan.commit && expected.is_none() {
        for marker in ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "rebase-merge", "rebase-apply", "sequencer"] {
            let marker = git(&path, &["rev-parse", "--path-format=absolute", "--git-path", marker])?;
            ensure!(!Path::new(&marker).try_exists()?, "Im Ziel-Checkout läuft bereits eine Git-Operation");
        }
        let tree = git(&path, &["write-tree"]).context("Gestagte Änderungen sind nicht commitbar")?;
        ensure!(tree != git(&path, &["rev-parse", &format!("{head}^{{tree}}")])?, "Keine gestagten Änderungen zum Committen");
        let message = plan.commit_message.as_deref().unwrap_or("Commit staged changes");
        let commit = publication::commit_tree(&path, &path, &["commit-tree", &tree, "-p", &head, "-m", message])?;
        ensure!(target_head(&path, &branch)? == head, "Zielbranch wurde inzwischen verändert");
        git(&path, &["update-ref", "-m", "YoloForge: commit staged changes", &branch, &commit, &head])?;
        head = commit;
        steps.push(format!("Commit: {head}"));
    }
    if let Some(tag) = &tag {
        ensure!(target_head(&path, &branch)? == head, "Zielbranch wurde inzwischen verändert");
        git(&path, &["update-ref", tag, &head, &"0".repeat(head.len())]).context("Tag konnte nicht sicher angelegt werden")?;
        steps.push(format!("Tag: {}", tag.trim_start_matches("refs/tags/")));
    }
    if let Some((remote, dest)) = push {
        ensure!(target_head(&path, &branch)? == head, "Zielbranch wurde inzwischen verändert");
        let branch_spec = format!("{head}:{dest}");
        let mut args = vec!["-c", "protocol.allow=never", "-c", "protocol.ssh.allow=always", "-c", "protocol.https.allow=always",
            "push", "--no-follow-tags", "--no-mirror", "--no-force", "--no-prune", "--no-push-option", "--recurse-submodules=no", "--no-verify"];
        let tag_spec;
        if tag.is_some() { args.push("--atomic"); }
        args.extend(["--", &remote, &branch_spec]);
        if let Some(tag) = &tag {
            tag_spec = format!("{head}:{tag}");
            args.push(&tag_spec);
        }
        git_network(&path, &args).context("Push nicht bestätigt; Remote prüfen, bevor erneut veröffentlicht wird")?;
        steps.push(format!("Push: {remote}/{dest}"));
    }
    Ok(())
}

pub fn execute(journal: &Journal, record: &mut Record, expected: Option<&str>) -> (bool, String) {
    let mut steps = vec![];
    let result = (|| {
        ensure!(!record.target_git_started, "Git-Schritte wurden unterbrochen; Ziel-Checkout und Remote vor Wiederholung prüfen");
        let spec = record.command.workspace_spec.clone().context("Ziel-Checkout fehlt")?;
        record.target_git_started = true;
        journal.save(record)?;
        execute_inner(&spec, expected, &mut steps)
    })();
    if let Err(error) = &result {
        steps.push(format!("Git-Schritte angehalten: {error}"));
    }
    let mut summary = steps.join("; ");
    let mut end = summary.len().min(1024);
    while !summary.is_char_boundary(end) { end -= 1; }
    summary.truncate(end);
    (result.is_ok(), summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_bumps_preserve_prefix_and_ignore_non_release_tags() {
        let tags = "v1.9.4\nv1.10.0\nv42.0.0-rc.1\nv8.03.0\nrelease\n";
        assert_eq!(next_tag(tags, &TargetTag::Patch).unwrap(), "v1.10.1");
        assert_eq!(next_tag(tags, &TargetTag::Minor).unwrap(), "v1.11.0");
        assert_eq!(next_tag(tags, &TargetTag::Major).unwrap(), "v2.0.0");
        assert_eq!(next_tag("1.0.0", &TargetTag::Patch).unwrap(), "1.0.1");
        assert_eq!(next_tag("nightly", &TargetTag::Patch).unwrap(), "v0.1.0");
    }

    #[test]
    fn direct_plan_commits_only_the_index_and_tags_the_target_commit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        git(path, &["init", "-b", "main"]).unwrap();
        git(path, &["config", "user.name", "Test"]).unwrap();
        git(path, &["config", "user.email", "test@example.invalid"]).unwrap();
        git(path, &["commit", "--allow-empty", "-m", "Initial"]).unwrap();
        git(path, &["tag", "v2.3.4"]).unwrap();
        fs::write(path.join("result"), "staged").unwrap();
        git(path, &["add", "result"]).unwrap();
        fs::write(path.join("result"), "unstaged").unwrap();
        fs::write(path.join("private"), "untracked").unwrap();
        let mut spec = WorkspaceSpec {
            target_git: Some(TargetGitPlan { commit: true, tag: Some(TargetTag::Patch), push: true,
                commit_message: Some("Requested message".into()), remote: None }),
            setup: Some(ProjectSetup { repository_source: path.display().to_string(), runner_id: RunnerId::new(),
                repository_path: String::new(), service_registration: String::new(), harness: None }),
            checkpoint: None, project_id: ProjectId::new(), branch: "factory/unused".into(), base_branch: "main".into(),
        };
        let before = git(path, &["rev-parse", "HEAD"]).unwrap();
        let mut steps = vec![];
        assert!(execute_inner(&spec, None, &mut steps).unwrap_err().to_string().contains("Upstream"));
        assert_eq!(git(path, &["rev-parse", "HEAD"]).unwrap(), before);
        assert_eq!(git(path, &["tag", "--list"]).unwrap(), "v2.3.4");

        git(path, &["remote", "add", "upstream", "https://example.invalid/project"]).unwrap();
        git(path, &["config", "branch.main.remote", "upstream"]).unwrap();
        git(path, &["config", "branch.main.merge", "refs/heads/release"]).unwrap();
        assert_eq!(push_destination(path, &spec, spec.target_git.as_ref().unwrap(), "refs/heads/main").unwrap(),
            ("upstream".into(), "refs/heads/release".into()));
        git(path, &["config", "remote.upstream.mirror", "true"]).unwrap();
        assert!(push_destination(path, &spec, spec.target_git.as_ref().unwrap(), "refs/heads/main").is_err());
        spec.target_git.as_mut().unwrap().push = false;
        execute_inner(&spec, None, &mut steps).unwrap();
        assert_eq!(git(path, &["show", "HEAD:result"]).unwrap(), "staged");
        assert_eq!(git(path, &["log", "-1", "--format=%s"]).unwrap(), "Requested message");
        assert_eq!(git(path, &["rev-parse", "HEAD"]).unwrap(), git(path, &["rev-parse", "v2.3.5"]).unwrap());
        assert_eq!(git(path, &["status", "--porcelain"]).unwrap(), "M result\n?? private");
        assert_eq!(git(path, &["branch", "--list", "factory/*"]).unwrap(), "");

        spec.target_git = Some(TargetGitPlan { tag: Some(TargetTag::Patch), ..Default::default() });
        assert!(execute_inner(&spec, Some(&before), &mut vec![]).is_err());
        assert_eq!(git(path, &["tag", "--list"]).unwrap(), "v2.3.4\nv2.3.5");
    }
}
