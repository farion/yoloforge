//! Publish a successful local task via an immutable snapshot and guarded fast-forward.
//! The execution worktree and its index are never modified.
use crate::{
    journal::{Journal, Record},
    workspace::{self, git, git_env, git_path},
    Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub(crate) struct PublicationConflict(pub String);
impl std::fmt::Display for PublicationConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PublicationConflict {}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Plan {
    /// Original execution snapshot, distinct from the combined publication commit.
    /// Older journals used commit for both.
    #[serde(default)]
    snapshot: Option<String>,
    #[serde(default)]
    merger_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery_failed_at: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    recovery_reported: bool,
    stream: StreamId,
    epoch: u64,
    target: PathBuf,
    repository: PathBuf,
    branch: String,
    parent: String,
    commit: String,
    published: bool,
}
fn save(j: &Journal, id: CommandId, p: &Plan) -> Result<()> {
    j.db.execute("INSERT INTO publications(command_id,data) VALUES(?1,?2) ON CONFLICT(command_id) DO UPDATE SET data=excluded.data",params![id.to_string(),serde_json::to_string(p)?])?;
    Ok(())
}

fn merger_parent(record: &Record) -> Result<Option<CommandId>> {
    let Some(spec) = record.command.agent_spec.as_ref().filter(|s| s.role == MERGER_ROLE) else { return Ok(None); };
    Ok(Some(spec.prompt.lines().next().and_then(|line| line.strip_prefix(MERGE_PARENT_PREFIX))
        .context("Merger source command missing")?.parse().map_err(|_| anyhow::anyhow!("Invalid Merger source command"))?))
}

fn source_plan(j: &Journal, record: &Record, source: CommandId) -> Result<Plan> {
    let raw: String = j.db.query_row("SELECT data FROM publications WHERE command_id=?1",
        [source.to_string()], |r| r.get(0)).context("Original publication snapshot unavailable on this runner")?;
    let plan: Plan = serde_json::from_str(&raw)?;
    ensure!(!plan.published && plan.stream == record.command.stream_id && plan.epoch == record.command.ownership_epoch,
        "Merger source no longer belongs to this execution");
    Ok(plan)
}

fn prepare_merger(j: &Journal, record: &Record, source: CommandId) -> Result<()> {
    let mut plan = source_plan(j, record, source)?;
    let snapshot = plan.snapshot.as_deref().context("Original publication snapshot missing")?;
    let snapshot_ref = format!("refs/yoloforge/publication-snapshots/{source}");
    ensure!(git(&plan.repository, &["rev-parse", &snapshot_ref])? == snapshot, "Original publication snapshot changed");
    let target_ref = format!("refs/yoloforge/merger-targets/{source}");
    let managed = record.command.workspace_spec.as_ref().and_then(|s| s.setup.as_ref())
        .is_some_and(|s| s.repository_source == "factory:managed");
    let current = if managed {
        git(&plan.repository, &["rev-parse", &format!("refs/heads/{}", plan.branch)])?
    } else {
        let current = git(&plan.target, &["rev-parse", "HEAD"])?;
        target_at(&plan.target, &plan.branch, &current)?;
        git(&plan.repository, &["fetch", "--refmap=", "--no-tags", "--no-write-fetch-head", &git_path(&plan.target)?, &current])?;
        current
    };
    match &plan.merger_target {
        Some(pinned) => ensure!(pinned == &current, "Target branch changed after Merger was prepared"),
        None => {
            plan.merger_target = Some(current.clone());
            save(j, source, &plan)?;
        }
    }
    let pinned = git(&plan.repository, &["for-each-ref", "--format=%(objectname)", &target_ref])?;
    if pinned.is_empty() {
        git(&plan.repository, &["update-ref", &target_ref, &current, &"0".repeat(current.len())])?;
    } else {
        ensure!(pinned == current, "Target branch changed after Merger was prepared");
    }
    Ok(())
}

pub(crate) fn execution_workspace(j: &Journal, r: &Record, args: &Args, prepare: bool) -> Result<Workspace> {
    let root = args.workspace_root.as_deref().context("Workspace root missing")?;
    let Some(source) = merger_parent(r)? else {
        return workspace::existing(j, &r.command, args.repositories.as_deref(), root);
    };
    let plan = source_plan(j, r, source)?;
    let path = root.join(format!("m-{}", r.command.id));
    if prepare {
        prepare_merger(j, r, source)?;
        if !path.exists() {
            let snapshot = plan.snapshot.as_deref().context("Original publication snapshot missing")?;
            git(&plan.repository, &["worktree", "add", "--detach", &git_path(&path)?, snapshot])?;
        }
    }
    let path = fs::canonicalize(path)?;
    let common = |directory: &Path| -> Result<PathBuf> {
        Ok(fs::canonicalize(git(directory, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?)?)
    };
    ensure!(common(&path)? == common(&plan.repository)?
        && fs::canonicalize(git(&path, &["rev-parse", "--show-toplevel"])?)? == path,
        "Merger workspace identity changed");
    ensure!(git(&path, &["symbolic-ref", "-q", "HEAD"]).is_err(), "Merger workspace must stay detached");
    Ok(Workspace {
        path: git_path(&path)?.into_owned(),
        branch: r.command.workspace_spec.as_ref().context("Workspace specification missing")?.branch.clone(),
        base_commit: plan.snapshot.context("Original publication snapshot missing")?,
    })
}

fn verify_merger(j: &Journal, record: &Record, args: &Args) -> Result<()> {
    let Some(source) = merger_parent(record)? else { return Ok(()); };
    prepare_merger(j, record, source)?;
    let workspace = execution_workspace(j, record, args, false)?;
    let path = Path::new(&workspace.path);
    ensure!(git(path, &["status", "--porcelain", "--untracked-files=normal"])?.is_empty(),
        "{MERGER_UNCOMMITTED_RESULT}");
    for reference in [format!("refs/yoloforge/publication-snapshots/{source}"), format!("refs/yoloforge/merger-targets/{source}")] {
        git(path, &["merge-base", "--is-ancestor", &reference, "HEAD"])
            .context("Merger resolution must contain both original results")?;
    }
    Ok(())
}

fn check_merger_target(j: &Journal, r: &Record, current: &str, published: Option<&str>) -> Result<()> {
    if let Some(source) = merger_parent(r)? {
        let pinned = source_plan(j, r, source)?.merger_target.context("Merger target commit missing")?;
        ensure!(current == pinned || published == Some(current),
            "Target branch advanced after Merger preparation; resolution retained without publication");
    }
    Ok(())
}
fn target_at(target: &Path, branch: &str, parent: &str) -> Result<()> {
    ensure!(
        git(target, &["symbolic-ref", "HEAD"])? == format!("refs/heads/{branch}"),
        "Projektordner steht auf einem anderen Branch; nichts ueberschrieben"
    );
    ensure!(
        git(target, &["rev-parse", "HEAD"])? == parent,
        "Projektstand wurde inzwischen geaendert; Ergebnis bleibt im Arbeitsbereich"
    );
    Ok(())
}

fn unchanged_target(target: &Path, branch: &str, parent: &str) -> Result<()> {
    target_at(target, branch, parent)?;
    ensure!(
        git(target, &["status", "--porcelain", "--untracked-files=all"])?.is_empty(),
        "Projektordner enthaelt eigene Aenderungen; nichts ueberschrieben"
    );
    Ok(())
}

fn protect_local_edits(target: &Path, parent: &str, commit: &str) -> Result<()> {
    // NUL separators preserve whitespace and newlines in Git paths. Disable
    // rename detection so both ends of a rename are protected.
    let changed = git(target, &["diff", "--no-renames", "--name-only", "-z", parent, commit, "--"])?;
    let changed: Vec<_> = changed.split_terminator('\0').map(str::to_lowercase).collect();
    for args in [
        &["diff", "--no-renames", "--name-only", "-z", "--"][..],
        &["diff", "--cached", "--no-renames", "--name-only", "-z", "--"],
        &["ls-files", "--others", "--exclude-standard", "-z"],
    ] {
        let local = git(target, args)?;
        for local in local.split_terminator('\0').map(str::to_lowercase) {
            ensure!(
                !changed.iter().any(|result| *result == local
                    || result.strip_prefix(&local).is_some_and(|rest| rest.starts_with('/'))
                    || local.strip_prefix(result).is_some_and(|rest| rest.starts_with('/'))),
                "Lokale Änderungen überschneiden sich mit dem Ergebnis; die Übernahme bleibt vorbereitet, nichts wurde überschrieben."
            );
        }
    }
    Ok(())
}
fn safe_checkout_filters(target: &Path) -> Result<()> {
    ensure!(!git(target, &["config", "--list", "--name-only"])?.lines()
        .any(|key| key.starts_with("filter.") &&
            (key.ends_with(".clean") || key.ends_with(".smudge") || key.ends_with(".process"))),
        "Lokaler Checkout verwendet Git-Filter; automatische Übernahme nicht möglich.");
    Ok(())
}

fn initialize_mirror(root: &Path, selected: &Path, spec: &WorkspaceSpec) -> Result<Workspace> {
    let import = spec.checkpoint.as_ref().context("Return checkpoint was not pinned")?;
    let parent = selected.parent().context("Mirror parent missing")?;
    let parent = fs::canonicalize(parent).context("Der Elternordner der lokalen Spiegelkopie fehlt.")?;
    let name = selected.file_name().context("Mirror folder name missing")?;
    let target = parent.join(name);
    ensure!(
        (target == selected || git_path(&target)? == git_path(selected)?)
            && fs::symlink_metadata(&target).is_err(),
        "Der Pfad der lokalen Spiegelkopie ist nicht neu oder nicht kanonisch.");
    let temp = tempfile::tempdir_in(&parent)?;
    git(temp.path(), &["init", "-b", &spec.base_branch])?;
    safe_checkout_filters(temp.path())?;
    let commit = crate::checkpoint::import_objects(temp.path(), root, import)?;
    git(temp.path(), &["reset", "--hard", &commit])?;
    ensure!(git(temp.path(), &["symbolic-ref", "HEAD"])?
        == format!("refs/heads/{}", spec.base_branch), "Mirror branch differs");
    let staged = temp.keep();
    fs::rename(&staged, &target)?;
    factory_platform::sync_dir(&parent)?;
    Ok(Workspace {
        path: selected.to_str().context("Non-UTF8 mirror path")?.into(),
        branch: spec.base_branch.clone(),
        base_commit: commit,
    })
}

pub(crate) fn return_checkpoint(root: &Path, spec: &WorkspaceSpec) -> Result<Workspace> {
    let setup = spec.setup.as_ref().context("Return checkout was not pinned")?;
    let import = spec.checkpoint.as_ref().context("Return checkpoint was not pinned")?;
    ensure!(setup.repository_source == CHECKPOINT_RETURN, "Not a checkpoint return");
    let selected = Path::new(&setup.repository_path);
    ensure!(selected.is_absolute(), "Der lokale Projektordner ist nicht absolut.");
    if matches!(fs::symlink_metadata(selected), Err(error) if error.kind() == std::io::ErrorKind::NotFound) {
        return initialize_mirror(&fs::canonicalize(root)?, selected, spec);
    }
    ensure!(fs::symlink_metadata(selected)?.is_dir(),
        "Der lokale Projektordner wurde durch einen Link ersetzt.");
    let target = fs::canonicalize(selected)?;
    ensure!(fs::canonicalize(git(&target, &["rev-parse", "--show-toplevel"])?)? == target,
        "Der lokale Projektpfad ist nicht der Git-Checkout.");
    // Checkout filters can run arbitrary commands on data added to a remote tree.
    safe_checkout_filters(&target)?;
    let head = git(&target, &["rev-parse", "HEAD"])?;
    unchanged_target(&target, &spec.base_branch, &head)?;
    let commit = crate::checkpoint::import_objects(&target, &fs::canonicalize(root)?, import)?;
    if git(&target, &["merge-base", "--is-ancestor", &commit, &head]).is_err() {
        git(&target, &["merge-base", "--is-ancestor", &head, &commit])
            .context("Lokaler und übertragener Stand sind auseinanderentwickelt; nichts überschrieben.")?;
        unchanged_target(&target, &spec.base_branch, &head)?;
        git(&target, &["-c", "merge.autoStash=false", "-c", "submodule.recurse=false",
            "merge", "--ff-only", "--no-overwrite-ignore", &commit])
            .context("Fast-forward des lokalen Checkouts fehlgeschlagen; lokale Dateien wurden nicht erzwungen überschrieben.")?;
    }
    Ok(Workspace { path: target.display().to_string(), branch: spec.base_branch.clone(), base_commit: commit })
}
pub fn publish(j: &Journal, r: &Record, args: &Args) -> Result<Option<PathBuf>> {
    let summary = match &r.result {
        Some(CommandOutput::Agent { agent }) => &agent.summary,
        _ => "",
    };
    publish_with_summary(j, r, args, summary)
}

fn summary_commit_message(summary: &str) -> Option<String> {
    let summary: String = summary.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .take(12_000)
        .collect();
    let heading = summary.lines().find_map(|line| {
        let line = line.trim().trim_start_matches(['#', '-', '*', '>']).trim().trim_end_matches('*').trim_end();
        (!line.is_empty()).then_some(line)
    });
    heading.map(|heading| {
        if summary.trim() == heading && heading.chars().count() <= 72 {
            heading.into()
        } else {
            let mut subject: String = heading.chars().take(72).collect();
            if heading.chars().count() > 72 {
                subject = format!("{}…", subject.chars().take(71).collect::<String>().trim_end());
            }
            format!("{}\n\n{}", subject.trim_end(), summary.trim())
        }
    })
}
fn managed_commit(repository: &Path, args: &[&str]) -> Result<String> {
    git_env(repository, args, &[
        ("GIT_AUTHOR_NAME", "YoloForge"),
        ("GIT_AUTHOR_EMAIL", "factory@yoloforge.invalid"),
        ("GIT_COMMITTER_NAME", "YoloForge"),
        ("GIT_COMMITTER_EMAIL", "factory@yoloforge.invalid"),
    ])
}

fn publish_managed(j: &Journal, r: &Record, args: &Args, summary: &str, prepare_only: bool) -> Result<Option<PathBuf>> {
    let spec = r.command.workspace_spec.as_ref().context("Workspace specification missing")?;
    let root = args.workspace_root.as_deref().context("Workspace root missing")?;
    let workspace = execution_workspace(j, r, args, false)?;
    let work = PathBuf::from(&workspace.path);
    let repository = workspace::repository(spec, args.repositories.as_deref(), root)?;
    let previous: Option<String> = j.db.query_row(
        "SELECT data FROM publications WHERE command_id=?1",
        [r.command.id.to_string()],
        |row| row.get(0),
    ).optional()?;
    let mut plan: Plan = if let Some(raw) = previous {
        let plan: Plan = serde_json::from_str(&raw)?;
        ensure!(plan.target == work && plan.repository == repository
            && plan.branch == spec.base_branch, "Managed publication binding changed");
        if plan.published {
            return Ok(Some(work));
        }
        plan
    } else {
        ensure!(git(&work, &["ls-files", "--unmerged"])?.is_empty(),
            "Arbeitsbereich enthaelt ungeloeste Konflikte");
        let temp = tempfile::tempdir_in(root)?;
        let index = temp.path().join("index");
        let index = git_path(&index)?;
        let env = [("GIT_INDEX_FILE", index.as_ref())];
        git_env(&work, &["read-tree", "HEAD"], &env)?;
        git_env(&work, &["add", "--all", "--", "."], &env)?;
        ensure!(!git_env(&work, &["ls-files", "--stage"], &env)?.lines()
            .any(|line| line.starts_with("160000 ")),
            "Verschachtelte Repositories/Submodule werden nicht automatisch uebernommen");
        let tree = git_env(&work, &["write-tree"], &env)?;
        let (parent, snapshot_parent) = if let Some(source) = merger_parent(r)? {
            let pin = source_plan(j, r, source)?.merger_target.context("Merger target commit missing")?;
            (pin, git(&work, &["rev-parse", "HEAD"])?)
        } else {
            // A reviewed publication leaves the implementation worktree
            // untouched. Its next snapshot is relative to the last approved
            // worktree tree, not the original checkout or combined target tree.
            let previous: Option<String> = j.db.query_row(
                "SELECT json_extract(p.data,'$.snapshot') FROM publications p JOIN commands c ON c.id=p.command_id
                 WHERE json_extract(p.data,'$.stream')=?1 AND json_extract(p.data,'$.epoch')=?2
                   AND json_extract(p.data,'$.published')=1 AND json_extract(c.data,'$.command.agent_spec.role')=?3
                 ORDER BY p.rowid DESC LIMIT 1",
                params![r.command.stream_id.to_string(), r.command.ownership_epoch, APPROVED_PUBLICATION_ROLE],
                |row| row.get(0),
            ).optional()?.flatten();
            let base = previous.unwrap_or_else(|| workspace.base_commit.clone());
            (base.clone(), base)
        };
        let message = summary_commit_message(summary).unwrap_or_else(|| "Record task result".into());
        let snapshot = managed_commit(&repository, &[
            "commit-tree", &tree, "-p", &snapshot_parent, "-m", &message,
        ])?;
        let plan = Plan {
            snapshot: Some(snapshot.clone()), merger_target: None, recovery_failed_at: None,
            recovery_reported: false, stream: r.command.stream_id,
            epoch: r.command.ownership_epoch, target: work.clone(),
            repository: repository.clone(), branch: spec.base_branch.clone(),
            parent, commit: snapshot, published: false,
        };
        save(j, r.command.id, &plan)?;
        plan
    };
    let snapshot = plan.snapshot.as_ref().context("Managed snapshot missing")?.clone();
    let snapshot_ref = format!("refs/yoloforge/publication-snapshots/{}", r.command.id);
    let existing = git(&repository, &["for-each-ref", "--format=%(objectname)", &snapshot_ref])?;
    if existing.is_empty() {
        git(&repository, &["update-ref", &snapshot_ref, &snapshot, &"0".repeat(snapshot.len())])?;
    } else {
        ensure!(existing == snapshot, "Managed publication snapshot changed");
    }
    if prepare_only {
        return Ok(Some(work));
    }
    let base_ref = format!("refs/heads/{}", spec.base_branch);
    let current = git(&repository, &["rev-parse", &base_ref])?;
    check_merger_target(j, r, &current, Some(&plan.commit))?;
    if current != plan.parent && current != plan.commit {
        ensure!(plan.commit == snapshot, "Managed base changed after result merge");
        git(&repository, &["merge-base", "--is-ancestor", &workspace.base_commit, &current])
            .context("Managed project history diverged")?;
        let base = git(&repository, &["merge-base", &snapshot, &current])?;
        let (clean, output) = workspace::merge_tree(&repository, &base, &current, &snapshot)?;
        if !clean {
            return Err(PublicationConflict(
                "Parallele Ergebnisse im Factory-Repository stehen in Konflikt; beide Snapshots bleiben erhalten.".into()
            ).into());
        }
        let tree = output.lines().next().context("Merge tree missing")?;
        let message = git(&repository, &["show", "-s", "--format=%B", &snapshot])?;
        plan.parent = current;
        plan.commit = managed_commit(&repository, &[
            "commit-tree", tree, "-p", &plan.parent, "-p", &snapshot, "-m", &message,
        ])?;
        save(j, r.command.id, &plan)?;
    }
    let publication_ref = format!("refs/yoloforge/publications/{}", r.command.id);
    let existing = git(&repository, &["for-each-ref", "--format=%(objectname)", &publication_ref])?;
    if existing.is_empty() {
        git(&repository, &["update-ref", &publication_ref, &plan.commit, &"0".repeat(plan.commit.len())])?;
    } else {
        ensure!(existing == plan.commit, "Managed publication ref changed");
    }
    let current = git(&repository, &["rev-parse", &base_ref])?;
    if current != plan.commit {
        ensure!(current == plan.parent, "Managed project base changed during publication");
        git(&repository, &["update-ref", &base_ref, &plan.commit, &current])?;
    }
    if merger_parent(r)?.is_none() && !is_approved_publication(&r.command) {
        git(&work, &["reset", "--hard", &plan.commit])?;
        ensure!(git(&work, &["status", "--porcelain", "--untracked-files=all"])?.is_empty(),
            "Managed result worktree is not clean after publication");
    }
    plan.published = true;
    save(j, r.command.id, &plan)?;
    Ok(Some(work))
}

pub(crate) fn commit_tree(target: &Path, repository: &Path, args: &[&str]) -> Result<String> {
    fn identity(target: &Path, kind: &str) -> Result<(String, String)> {
        let ident = git(target, &["-c", "user.useConfigOnly=true", "var", kind])
            .context("Git-Identität im Zielrepository fehlt; bitte user.name und user.email konfigurieren")?;
        let (name, tail) = ident.rsplit_once(" <").context("Invalid Git identity")?;
        let (email, _) = tail.split_once("> ").context("Invalid Git identity")?;
        Ok((name.into(), email.into()))
    }
    let (author, author_email) = identity(target, "GIT_AUTHOR_IDENT")?;
    let (committer, committer_email) = identity(target, "GIT_COMMITTER_IDENT")?;
    git_env(repository, args, &[
        ("GIT_AUTHOR_NAME", &author),
        ("GIT_AUTHOR_EMAIL", &author_email),
        ("GIT_COMMITTER_NAME", &committer),
        ("GIT_COMMITTER_EMAIL", &committer_email),
    ])
}

pub fn publish_with_summary(j: &Journal, r: &Record, args: &Args, summary: &str) -> Result<Option<PathBuf>> {
    publish_or_prepare(j, r, args, summary, false)
}

pub(crate) fn is_approved_publication(command: &RunnerCommand) -> bool {
    command.agent_spec.as_ref().is_some_and(|s| s.role == APPROVED_PUBLICATION_ROLE)
}

pub(crate) fn prepare_for_review(j: &Journal, r: &Record, args: &Args, summary: &str) -> Result<Option<PathBuf>> {
    publish_or_prepare(j, r, args, summary, true)
}

pub(crate) fn publish_approved(j: &Journal, r: &Record, args: &Args) -> Result<()> {
    let source = r.command.agent_spec.as_ref().context("Missing publication source")?
        .prompt.parse::<CommandId>().map_err(|_| anyhow::anyhow!("Invalid publication source"))?;
    let plan = source_plan(j, r, source)?;
    let snapshot = plan.snapshot.as_ref().context("Missing reviewed snapshot")?;
    let reference = format!("refs/yoloforge/publication-snapshots/{source}");
    ensure!(git(&plan.repository, &["rev-parse", &reference])? == *snapshot, "Reviewed snapshot changed");
    j.db.execute("INSERT OR IGNORE INTO publications(command_id,data) VALUES(?1,?2)",
        params![r.command.id.to_string(), serde_json::to_string(&plan)?])?;
    publish_with_summary(j, r, args, "")?.context("Missing publication destination")?;
    Ok(())
}

fn publish_or_prepare(j: &Journal, r: &Record, args: &Args, summary: &str, prepare_only: bool) -> Result<Option<PathBuf>> {
    // Publication already journaled may be recovering after it advanced the
    // target. In that case the existing plan is the durable proof.
    let already_planned: bool = j.db.query_row("SELECT EXISTS(SELECT 1 FROM publications WHERE command_id=?1)",
        [r.command.id.to_string()], |row| row.get(0))?;
    if !already_planned {
        verify_merger(j, r, args)?;
    }
    let spec = r
        .command
        .workspace_spec
        .as_ref()
        .context("Workspace specification missing")?;
    let Some(setup) = &spec.setup else {
        return Ok(None);
    };
    if setup.repository_source == "factory:managed" && setup.repository_path.is_empty() {
        return publish_managed(j, r, args, summary, prepare_only);
    }
    // Remote URLs and Factory-managed storage have no user-selected local output folder.
    let source = Path::new(&setup.repository_source);
    if !source.is_absolute() || !setup.repository_path.is_empty() {
        return Ok(None);
    }
    let root = args
        .workspace_root
        .as_deref()
        .context("Workspace root missing")?;
    let workspace = execution_workspace(j, r, args, false)?;
    let repository = workspace::repository(spec, args.repositories.as_deref(), root)?;
    let target = fs::canonicalize(source).context("Projektordner fehlt")?;
    ensure!(
        fs::canonicalize(git(&target, &["rev-parse", "--show-toplevel"])?)? == target,
        "Projektordner ist kein eigenstaendiges Repository"
    );
    let previous: Option<String> =
        j.db.query_row(
            "SELECT data FROM publications WHERE command_id=?1",
            [r.command.id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let mut plan: Plan = if let Some(raw) = previous {
        let plan: Plan = serde_json::from_str(&raw)?;
        ensure!(
            plan.target == target
                && plan.repository == repository
                && plan.branch == spec.base_branch,
            "Publication binding changed"
        );
        if plan.published {
            return Ok(Some(target));
        }
        plan
    } else {
        let previous:Option<String>=j.db.query_row("SELECT COALESCE(json_extract(data,'$.snapshot'),json_extract(data,'$.commit')) FROM publications WHERE json_extract(data,'$.stream')=?1 AND json_extract(data,'$.epoch')=?2 AND json_extract(data,'$.published')=1 ORDER BY rowid DESC LIMIT 1",params![r.command.stream_id.to_string(),r.command.ownership_epoch],|row|row.get(0)).optional()?;
        let current = if prepare_only { workspace.base_commit.clone() } else { git(&target, &["rev-parse", "HEAD"])? };
        check_merger_target(j, r, &current, None)?;
        let parent = if merger_parent(r)?.is_some() { current.clone() } else {
            previous.unwrap_or_else(|| workspace.base_commit.clone())
        };
        if !prepare_only {
            target_at(&target, &spec.base_branch, &current)?;
        }
        if spec.target_git.is_some() {
            unchanged_target(&target, &spec.base_branch, &current)?;
        }
        let work = Path::new(&workspace.path);
        ensure!(
            git(work, &["ls-files", "--unmerged"])?.is_empty(),
            "Arbeitsbereich enthaelt ungeloeste Konflikte"
        );
        let temp = tempfile::tempdir_in(root)?;
        let index = temp.path().join("index");
        let index = git_path(&index)?;
        let env = [("GIT_INDEX_FILE", index.as_ref())];
        git_env(work, &["read-tree", "HEAD"], &env)?;
        git_env(work, &["add", "--all", "--", "."], &env)?;
        ensure!(
            !git_env(work, &["ls-files", "--stage"], &env)?
                .lines()
                .any(|line| line.starts_with("160000 ")),
            "Verschachtelte Repositories/Submodule werden nicht automatisch uebernommen"
        );
        let tree = git_env(work, &["write-tree"], &env)?;
        if spec.target_git.is_some() {
            ensure!(tree != git(work, &["rev-parse", &format!("{parent}^{{tree}}")])?,
                "Arbeitsbereich enthält keine Änderungen für die angeforderte Übernahme");
        }
        let head = git(work, &["rev-parse", "HEAD"])?;
        // A targeted follow-up may have merged the observed target into its
        // execution branch and committed a real resolution. Preserve that
        // ancestry so Git need not rediscover the already resolved conflict.
        let mut parents = vec!["-p", parent.as_str()];
        if head != parent && git(work, &["merge-base", "--is-ancestor", &current, &head]).is_ok() {
            parents.extend(["-p", head.as_str()]);
        }
        let mut commit_args = vec!["commit-tree", tree.as_str()];
        commit_args.extend(parents);
        let requested_message = spec.target_git.as_ref().and_then(|plan| plan.commit_message.clone());
        let message = if let Some(message) = requested_message.or_else(|| summary_commit_message(summary)) {
            message
        } else {
            let changes = git(work, &["diff", "--stat", &parent, &tree, "--"])?;
            if changes.is_empty() {
                "Record task result".into()
            } else {
                format!("Update project files\n\n{changes}")
            }
        };
        commit_args.extend(["-m", message.as_str()]);
        let commit = commit_tree(&target, work, &commit_args)?;
        let plan = Plan {
            snapshot: Some(commit.clone()),
            merger_target: None,
            recovery_failed_at: None,
            recovery_reported: false,
            stream: r.command.stream_id,
            epoch: r.command.ownership_epoch,
            target: target.clone(),
            repository: repository.clone(),
            branch: spec.base_branch.clone(),
            parent,
            commit,
            published: false,
        };
        save(j, r.command.id, &plan)?;
        plan
    };
    if let Some(snapshot) = &plan.snapshot {
        // Retain the immutable source even if combining it reports conflicts.
        let reference = format!("refs/yoloforge/publication-snapshots/{}", r.command.id);
        let existing = git(
            &repository,
            &["for-each-ref", "--format=%(objectname)", &reference],
        )?;
        if existing.is_empty() {
            git(
                &repository,
                &[
                    "update-ref",
                    &reference,
                    snapshot,
                    &"0".repeat(snapshot.len()),
                ],
            )?;
        } else {
            ensure!(existing == *snapshot, "Publication snapshot ref changed");
        }
        if prepare_only {
            return Ok(Some(target));
        }
        let current = git(&target, &["rev-parse", "HEAD"])?;
        check_merger_target(j, r, &current, Some(&plan.commit))?;
        if current != plan.parent && current != plan.commit {
            ensure!(
                plan.commit == *snapshot,
                "Projektstand nach gepinnter Zusammenfuehrung geaendert; Ergebnis erhalten"
            );
            target_at(&target, &plan.branch, &current)?;
            git(
                &repository,
                &[
                    "fetch",
                    "--refmap=",
                    "--no-tags",
                    "--no-write-fetch-head",
                    "--no-auto-maintenance",
                    &git_path(&target)?,
                    &current,
                ],
            )?;
            git(
                &repository,
                &[
                    "merge-base",
                    "--is-ancestor",
                    &workspace.base_commit,
                    &current,
                ],
            )
            .context("Projektgeschichte ist abgezweigt; keine automatische Zusammenfuehrung")?;
            let base = git(&repository, &["merge-base", snapshot, &current])?;
            let (clean, output) = workspace::merge_tree(&repository, &base, &current, snapshot)?;
            if !clean {
                let details = output.lines().skip(1).collect::<Vec<_>>().join("\n");
                let details = details
                    .chars()
                    .filter(|c| !c.is_control() || *c == '\n')
                    .take(4096)
                    .collect::<String>();
                return Err(PublicationConflict(format!("Paralleler Ergebniskonflikt. Ziel: {}; Basis: {}; Zielcommit: {}; Ergebnissnapshot: {}. Betroffene Dateien / Git-Diagnose:\n{}\nNacharbeit in derselben Session: Zielcommit in den Ausfuehrungsbranch mergen, Konflikte dort loesen und die Aufloesung committen; beide Ergebnisse bewahren und Tests ausfuehren. Nicht den Projektordner bearbeiten. Danach neuer Folgeauftrag statt Aenderung des gepinnten Snapshots.", target.display(), base, current, snapshot, details)).into());
            }
            let tree = output.lines().next().context("Merge tree missing")?;
            let message = git(&repository, &["show", "-s", "--format=%B", snapshot])?;
            let commit = commit_tree(
                &target,
                &repository,
                &[
                    "commit-tree",
                    tree,
                    "-p",
                    &current,
                    "-p",
                    snapshot,
                    "-m",
                    &message,
                ],
            )?;
            plan.parent = current;
            plan.commit = commit;
            save(j, r.command.id, &plan)?;
        }
    }
    let reference = format!("refs/yoloforge/publications/{}", r.command.id);
    let existing = git(
        &repository,
        &["for-each-ref", "--format=%(objectname)", &reference],
    )?;
    if existing.is_empty() {
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
        ensure!(existing == plan.commit, "Publication ref changed");
    }
    promote_local(&target, &repository, &plan, &reference, spec.target_git.is_some())?;
    plan.published = true;
    save(j, r.command.id, &plan)?;
    Ok(Some(target))
}

fn promote_local(target: &Path, repository: &Path, plan: &Plan, reference: &str, explicit_git: bool) -> Result<()> {
    let target_head = git(target, &["rev-parse", "HEAD"])?;
    if target_head != plan.commit {
        target_at(&target, &plan.branch, &plan.parent)?;
        git(
            &target,
            &[
                "fetch",
                "--no-tags",
                "--refmap=",
                "--no-write-fetch-head",
                "--no-auto-maintenance",
                &git_path(&repository)?,
                &reference,
            ],
        )?;
        // Recheck after fetch. Git itself refuses overlapping dirty/untracked files at checkout.
        target_at(&target, &plan.branch, &plan.parent)?;
        if explicit_git {
            unchanged_target(&target, &plan.branch, &plan.parent)?;
        } else {
            protect_local_edits(&target, &plan.parent, &plan.commit)?;
        }
        git(
            &target,
            &[
                "-c",
                "merge.autoStash=false",
                "-c",
                "submodule.recurse=false",
                "merge",
                "--ff-only",
                "--no-overwrite-ignore",
                "--no-edit",
                &plan.commit,
            ],
        )?;
    }
    ensure!(
        git(&target, &["symbolic-ref", "HEAD"])? == format!("refs/heads/{}", plan.branch),
        "Target branch changed during publication"
    );
    git(&target, &["merge-base", "--is-ancestor", &plan.commit, "HEAD"])
        .context("Target branch does not contain the published result")?;
    let base = format!("refs/heads/{}", plan.branch);
    let current = git(&repository, &["rev-parse", &base])?;
    if current != plan.commit {
        ensure!(
            current == plan.parent
                || (plan.snapshot.is_some()
                    && git(
                        &repository,
                        &["merge-base", "--is-ancestor", &current, &plan.parent]
                    )
                    .is_ok()),
            "Managed project base changed during publication"
        );
        git(&repository, &["update-ref", &base, &plan.commit, &current])?;
    }
    Ok(())
}

pub(crate) fn published_unchanged_tree(journal: &Journal, id: CommandId) -> Result<bool> {
    let raw: Option<String> = journal.db.query_row("SELECT data FROM publications WHERE command_id=?1",
        [id.to_string()], |row| row.get(0)).optional()?;
    let Some(raw) = raw else { return Ok(false); };
    let plan: Plan = serde_json::from_str(&raw)?;
    let Some(snapshot) = plan.snapshot else { return Ok(false); };
    let tree = git(&plan.repository, &["rev-parse", &format!("{snapshot}^{{tree}}")])?;
    let parent_tree = git(&plan.repository, &["rev-parse", &format!("{snapshot}^1^{{tree}}")])?;
    Ok(tree == parent_tree)
}

pub(crate) fn recovery_candidate(journal: &Journal, position: usize) -> Result<Option<CommandId>> {
    let candidates: i64 = journal.db.query_row(
        &format!("SELECT COUNT(*) {RECOVERY_CANDIDATES}"), [], |row| row.get(0),
    )?;
    if candidates == 0 { return Ok(None); }
    let id: String = journal.db.query_row(
        &format!("SELECT p.command_id {RECOVERY_CANDIDATES} ORDER BY p.rowid LIMIT 1 OFFSET ?1"),
        [position as u64 % candidates as u64], |row| row.get(0),
    )?;
    Ok(Some(id.parse()?))
}

const RECOVERY_CANDIDATES: &str = "FROM publications p JOIN commands c ON c.id=p.command_id
    WHERE json_extract(c.data,'$.reported')=1
      AND COALESCE(json_extract(c.data,'$.recovery_required'),0)=0
      AND COALESCE(json_extract(p.data,'$.recovery_reported'),0)=0
      AND json_extract(c.data,'$.result.agent.phase')='succeeded'
      AND json_extract(p.data,'$.snapshot') IS NOT NULL
      AND (instr(json_extract(c.data,'$.result.agent.summary'),'[yoloforge:publication-blocked]')=1
           OR instr(json_extract(c.data,'$.result.agent.summary'),
              'Ergebnis im Arbeitsbereich erhalten; automatische Uebernahme in den Projektordner blockiert:')>0)";

pub(crate) fn acknowledge_recovery(journal: &Journal, id: CommandId) -> Result<()> {
    journal.db.execute("UPDATE publications SET data=json_set(data,'$.recovery_reported',json('true'))
        WHERE command_id=?1", [id.to_string()])?;
    Ok(())
}

pub(crate) fn recover_prepared(
    journal: &Journal, id: CommandId, grant_id: RequestId,
) -> Result<Option<factory_protocol::ReportRecoveredPublication>> {
    use sha2::{Digest, Sha256};
    let Some(record) = journal.record(id)? else { return Ok(None); };
    let Some(spec) = record.command.workspace_spec.as_ref() else { return Ok(None); };
    let Some(setup) = spec.setup.as_ref() else { return Ok(None); };
    if !record.reported || record.recovery_required || record.command.requires_target_git()
        || !setup.repository_path.is_empty() || !Path::new(&setup.repository_source).is_absolute()
        || !matches!(&record.result, Some(CommandOutput::Agent { agent })
            if agent.phase == AgentPhase::Succeeded && publication_was_blocked(&agent.summary)
                && !agent.summary.starts_with(PUBLICATION_CONFLICT_PREFIX))
    {
        return Ok(None);
    }
    let raw: Option<String> = journal.db.query_row("SELECT data FROM publications WHERE command_id=?1",
        [id.to_string()], |row| row.get(0)).optional()?;
    let Some(raw) = raw else { return Ok(None); };
    let mut plan: Plan = serde_json::from_str(&raw)?;
    let Some(snapshot) = plan.snapshot.clone() else { return Ok(None); };
    ensure!(plan.stream == record.command.stream_id && plan.epoch == record.command.ownership_epoch
        && plan.branch == spec.base_branch && fs::canonicalize(&setup.repository_source)? == plan.target,
        "Recovery publication binding changed");
    let reference = format!("refs/yoloforge/publications/{id}");
    // Only finish a candidate that reached the durable publication ref.
    // Never read or snapshot today's execution worktree during recovery.
    if git(&plan.repository, &["for-each-ref", "--format=%(objectname)", &reference])? != plan.commit
        || git(&plan.repository, &["for-each-ref", "--format=%(objectname)",
            &format!("refs/yoloforge/publication-snapshots/{id}")])? != snapshot
    {
        return Ok(None);
    }
    git(&plan.repository, &["merge-base", "--is-ancestor", &snapshot, &plan.commit])?;
    let branch = format!("refs/heads/{}", plan.branch);
    let mut head = git(&plan.target, &["rev-parse", "--verify", &branch])?;
    if git(&plan.target, &["merge-base", "--is-ancestor", &plan.commit, &head]).is_err() {
        if plan.published || head != plan.parent { return Ok(None); }
        target_at(&plan.target, &plan.branch, &plan.parent)?;
        safe_checkout_filters(&plan.target)?;
        let state = git(&plan.target, &["status", "--porcelain", "-z", "--untracked-files=all", "--ignored=matching"])?;
        let fingerprint = format!("{head}:{:x}", Sha256::digest(state.as_bytes()));
        if plan.recovery_failed_at.as_deref() == Some(&fingerprint) { return Ok(None); }
        if let Err(error) = promote_local(&plan.target, &plan.repository, &plan, &reference, false) {
            plan.recovery_failed_at = Some(fingerprint);
            save(journal, id, &plan)?;
            return Err(error);
        }
        plan.published = true;
        save(journal, id, &plan)?;
        head = git(&plan.target, &["rev-parse", "--verify", &branch])?;
    }
    git(&plan.target, &["merge-base", "--is-ancestor", &plan.commit, &head])?;
    Ok(Some(factory_protocol::ReportRecoveredPublication {
        request_id: RequestId::new(), grant_id, ownership_epoch: plan.epoch, base_branch: plan.branch,
        snapshot_commit: snapshot, target_commit: head,
    }))
}

pub(crate) fn published_commit(j: &Journal, id: CommandId) -> Result<String> {
    let raw: String = j.db.query_row("SELECT data FROM publications WHERE command_id=?1",
        [id.to_string()], |row| row.get(0))?;
    let plan: Plan = serde_json::from_str(&raw)?;
    ensure!(plan.published, "Ergebnis wurde nicht übernommen");
    ensure!(git(&plan.target, &["rev-parse", &format!("{}^{{tree}}", plan.commit)])?
        != git(&plan.target, &["rev-parse", &format!("{}^{{tree}}", plan.parent)])?,
        "Übernahme enthält keine Änderungen; kein Tag oder Push ausgelöst");
    Ok(plan.commit)
}

/// The Merger publishes from a detached worktree, not from the stream's
/// original workspace. Export the confirmed commit rather than that old HEAD.
pub(crate) fn managed_export_commit(j: &Journal, command: &RunnerCommand, repository: &Path) -> Result<Option<String>> {
    let raw: Option<String> = j.db.query_row(
        "SELECT data FROM publications WHERE json_extract(data,'$.stream')=?1
         AND json_extract(data,'$.epoch')=?2 AND json_extract(data,'$.published')=1
         ORDER BY rowid DESC LIMIT 1",
        params![command.stream_id.to_string(), command.ownership_epoch],
        |row| row.get(0),
    ).optional()?;
    let Some(raw) = raw else { return Ok(None); };
    let plan: Plan = serde_json::from_str(&raw)?;
    let base = &command.workspace_spec.as_ref().context("Workspace specification missing")?.base_branch;
    ensure!(plan.branch == *base && workspace::common_dir(&plan.repository)? == repository,
        "Managed publication does not match the checkpoint repository");
    git(repository, &["merge-base", "--is-ancestor", &plan.commit, &format!("refs/heads/{base}")])
        .context("Published result is no longer on the managed base branch")?;
    Ok(Some(plan.commit))
}

pub(crate) fn verify_integrated_snapshot(record: &Record, snapshot: &str) -> Result<(String, String)> {
    ensure!(matches!(snapshot.len(), 40 | 64) && snapshot.bytes().all(|b| b.is_ascii_hexdigit()),
        "Expected a full Git commit ID");
    let spec = record.command.workspace_spec.as_ref().context("Workspace specification missing")?;
    let setup = spec.setup.as_ref().filter(|s| s.repository_path.is_empty())
        .context("No direct project checkout configured")?;
    let target = Path::new(&setup.repository_source);
    ensure!(target.is_absolute(), "No local project checkout configured");
    let source = format!("refs/yoloforge/consolidation/{}", record.command.stream_id);
    ensure!(git(target, &["rev-parse", "--verify", &format!("{source}^{{commit}}")])? == snapshot,
        "Snapshot does not match the archived source for this stream");
    let branch = format!("refs/heads/{}", spec.base_branch);
    let commit = git(target, &["rev-parse", "--verify", &format!("{branch}^{{commit}}")])?;
    git(target, &["merge-base", "--is-ancestor", snapshot, &commit])
        .context("Archived snapshot is not integrated into the configured target branch")?;
    Ok((spec.base_branch.clone(), commit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    struct Fixture {
        _dir: tempfile::TempDir,
        journal: Journal,
        args: Args,
        record: Record,
        work: PathBuf,
        target: PathBuf,
    }
    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        let path = dir.path().join("journal");
        let target = dir.path().join("project");
        let runner = RunnerId::new();
        let project = ProjectId::new();
        let stream = StreamId::new();
        let args = Args::parse_from([
            "runner",
            "--id",
            &runner.to_string(),
            "--name",
            "test",
            "--journal",
            path.to_str().unwrap(),
            "--workspace-root",
            root.to_str().unwrap(),
        ]);
        let command:RunnerCommand=serde_json::from_value(serde_json::json!({"id":CommandId::new(),"stream_id":stream,"runner_id":runner,"ownership_epoch":1,"kind":"run_agent","state":"accepted","created_unix_ms":1,"workspace_spec":{"project_id":project,"branch":format!("factory/{stream}"),"base_branch":"main","setup":{"runner_id":runner,"repository_source":target.to_str().unwrap(),"repository_path":"","service_registration":"","harness":null}}})).unwrap();
        let journal = Journal::open(&path, &args.server, runner).unwrap();
        let mut record = journal.receive(command).unwrap();
        crate::managed::ensure_repository(&journal, &mut record, &args)
            .await
            .unwrap();
        git(&target, &["config", "--local", "user.name", "Local Developer"]).unwrap();
        git(&target, &["config", "--local", "user.email", "developer@example.invalid"]).unwrap();
        let CommandOutput::Workspace { workspace } =
            workspace::prepare(&journal, &record.command, None, &root)
        else {
            panic!()
        };
        let work = PathBuf::from(workspace.path);
        fs::write(work.join(".gitignore"), "target/\n").unwrap();
        fs::create_dir(work.join("target")).unwrap();
        fs::write(work.join("target/build-artifact"), "exclude").unwrap();
        fs::write(work.join("result.txt"), "result").unwrap();
        Fixture {
            _dir: dir,
            journal,
            args,
            record,
            work,
            target,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dirty_target_does_not_prevent_review_and_merge_uses_only_the_reviewed_snapshot() {
        let f = fixture().await;
        let before = git(&f.target, &["rev-parse", "HEAD"]).unwrap();
        fs::write(f.target.join("result.txt"), "local edit").unwrap();
        prepare_for_review(&f.journal, &f.record, &f.args, "Reviewed result").unwrap();
        assert_eq!(git(&f.target, &["rev-parse", "HEAD"]).unwrap(), before);
        assert_eq!(fs::read_to_string(f.target.join("result.txt")).unwrap(), "local edit");
        assert!(published_commit(&f.journal, f.record.command.id).is_err());

        let mut command = f.record.command.clone();
        command.id = CommandId::new();
        command.agent_spec = Some(serde_json::from_value(serde_json::json!({
            "role": APPROVED_PUBLICATION_ROLE,
            "prompt": f.record.command.id.to_string(),
            "profile": ExecutionProfile::default()
        })).unwrap());
        let merge = f.journal.receive(command).unwrap();
        assert!(publish_approved(&f.journal, &merge, &f.args).is_err());
        assert_eq!(fs::read_to_string(f.target.join("result.txt")).unwrap(), "local edit");
        fs::remove_file(f.target.join("result.txt")).unwrap();
        fs::write(f.work.join("result.txt"), "post-review edit").unwrap();
        publish_approved(&f.journal, &merge, &f.args).unwrap();
        publish_approved(&f.journal, &merge, &f.args).unwrap();
        assert_eq!(fs::read_to_string(f.target.join("result.txt")).unwrap(), "result");
        assert_eq!(fs::read_to_string(f.work.join("result.txt")).unwrap(), "post-review edit");
    }
    fn parallel(f: &Fixture) -> (Record, PathBuf) {
        let mut command = f.record.command.clone();
        command.id = CommandId::new();
        command.stream_id = StreamId::new();
        command.workspace_spec.as_mut().unwrap().branch = format!("factory/{}", command.stream_id);
        let record = f.journal.receive(command).unwrap();
        let CommandOutput::Workspace { workspace } = workspace::prepare(
            &f.journal,
            &record.command,
            None,
            f.args.workspace_root.as_deref().unwrap(),
        ) else {
            panic!("parallel workspace failed")
        };
        (record, PathBuf::from(workspace.path))
    }

    fn merger_command(source: &RunnerCommand) -> RunnerCommand {
        let mut command = source.clone();
        command.id = CommandId::new();
        command.state = CommandState::Pending;
        command.agent_spec = Some(serde_json::from_value(serde_json::json!({
            "prompt": format!("{MERGE_PARENT_PREFIX}{}", source.id),
            "role": MERGER_ROLE,
            "profile": ExecutionProfile::default(),
        })).unwrap());
        command
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn factory_managed_results_advance_the_base_and_leave_a_clean_checkpoint_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        let journal_path = dir.path().join("journal");
        let runner = RunnerId::new();
        let project = ProjectId::new();
        let stream = StreamId::new();
        let args = Args::parse_from([
            "runner", "--id", &runner.to_string(), "--name", "test",
            "--journal", journal_path.to_str().unwrap(),
            "--workspace-root", root.to_str().unwrap(),
        ]);
        let command: RunnerCommand = serde_json::from_value(serde_json::json!({
            "id":CommandId::new(),"stream_id":stream,"runner_id":runner,
            "ownership_epoch":1,"kind":"run_agent","state":"accepted","created_unix_ms":1,
            "workspace_spec":{"project_id":project,"branch":format!("factory/{stream}"),
            "base_branch":"main","setup":{"runner_id":runner,
            "repository_source":"factory:managed","repository_path":"",
            "service_registration":"","harness":null}}
        })).unwrap();
        let journal = Journal::open(&journal_path, &args.server, runner).unwrap();
        let mut record = journal.receive(command).unwrap();
        crate::managed::ensure_repository(&journal, &mut record, &args).await.unwrap();
        let CommandOutput::Workspace { workspace } = workspace::prepare(
            &journal, &record.command, None, &root,
        ) else { panic!("managed workspace failed") };
        let work = PathBuf::from(workspace.path);
        fs::write(work.join("result.txt"), "remote result").unwrap();
        let mut parallel = record.command.clone();
        parallel.id = CommandId::new();
        parallel.stream_id = StreamId::new();
        parallel.workspace_spec.as_mut().unwrap().branch = format!("factory/{}", parallel.stream_id);
        let source = journal.receive(parallel).unwrap();
        let CommandOutput::Workspace { workspace: parallel_work } =
            workspace::prepare(&journal, &source.command, None, &root)
        else { panic!("parallel managed workspace failed") };
        let original = Path::new(&parallel_work.path);
        fs::write(original.join("result.txt"), "parallel result").unwrap();
        let published = publish_with_summary(&journal, &record, &args, "Remote result")
            .unwrap().unwrap();
        assert_eq!(published, work);
        assert_eq!(git(&work, &["status", "--porcelain", "--untracked-files=all"]).unwrap(), "");
        let repository = workspace::repository(
            record.command.workspace_spec.as_ref().unwrap(), None, &root,
        ).unwrap();
        assert_eq!(git(&repository, &["show", "main:result.txt"]).unwrap(), "remote result");
        assert_eq!(git(&work, &["rev-parse", "HEAD"]).unwrap(),
            git(&repository, &["rev-parse", "main"]).unwrap());
        assert!(publish_with_summary(&journal, &record, &args, "Remote result").is_ok());
        assert!(publish(&journal, &source, &args).unwrap_err().is::<PublicationConflict>());
        let merger = journal.receive(merger_command(&source.command)).unwrap();
        let isolated = execution_workspace(&journal, &merger, &args, true).unwrap();
        let isolated = Path::new(&isolated.path);
        let target = git(&repository, &["rev-parse", "main"]).unwrap();
        assert!(managed_commit(isolated, &["merge", "--no-edit", &target]).is_err());
        fs::write(isolated.join("result.txt"), "merged result").unwrap();
        git(isolated, &["add", "--", "result.txt"]).unwrap();
        managed_commit(isolated, &["commit", "-m", "resolve conflict"]).unwrap();
        publish(&journal, &merger, &args).unwrap();
        assert_eq!(git(&repository, &["show", "main:result.txt"]).unwrap(), "merged result");
        assert_eq!(fs::read_to_string(original.join("result.txt")).unwrap(), "parallel result");
        assert_eq!(managed_export_commit(&journal, &merger.command, &workspace::common_dir(&repository).unwrap()).unwrap(),
            Some(git(&repository, &["rev-parse", "main"]).unwrap()));
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn split_git_tags_the_published_commit_and_reconciles_without_a_second_tag() {
        let mut f = fixture().await;
        // This fixture simulates a locally owned execution, not recovery after
        // losing the accepted command's write-ahead journal.
        f.record.recovery_required = false;
        f.record.command.workspace_spec.as_mut().unwrap().target_git = Some(TargetGitPlan {
            commit: true,
            commit_message: Some("User-selected subject".into()),
            tag: Some(TargetTag::Patch),
            ..Default::default()
        });
        let before = git(&f.work, &["rev-parse", "HEAD"]).unwrap();
        fs::write(f.target.join("dirty"), "user file").unwrap();
        assert!(publish(&f.journal, &f.record, &f.args).is_err());
        assert!(published_commit(&f.journal, f.record.command.id).is_err());
        assert_eq!(git(&f.target, &["tag", "--list"]).unwrap(), "");
        fs::remove_file(f.target.join("dirty")).unwrap();
        publish(&f.journal, &f.record, &f.args).unwrap();
        let commit = published_commit(&f.journal, f.record.command.id).unwrap();
        assert_eq!(git(&f.target, &["log", "-1", "--format=%s"]).unwrap(), "User-selected subject");
        assert_eq!(git(&f.work, &["rev-parse", "HEAD"]).unwrap(), before);
        assert_ne!(before, commit);
        assert!(crate::target_git::execute(&f.journal, &mut f.record, Some(&commit)).0);
        assert_eq!(git(&f.target, &["rev-parse", "v0.1.0"]).unwrap(), commit);
        let mut replay = f.journal.receive(f.record.command.clone()).unwrap();
        assert!(crate::target_git::execute(&f.journal, &mut replay, Some(&commit)).0);
        assert_eq!(git(&f.target, &["tag", "--list"]).unwrap(), "v0.1.0");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn parallel_disjoint_results_combine_and_continuations_do_not_erase_other_work() {
        let f = fixture().await;
        let (mut second, work) = parallel(&f);
        fs::write(work.join("other.txt"), "second result").unwrap();
        let head = git(&work, &["rev-parse", "HEAD"]).unwrap();
        let index = git(&work, &["ls-files", "--stage"]).unwrap();
        publish_with_summary(&f.journal, &f.record, &f.args, "Add first result").unwrap();
        let first = git(&f.target, &["show", "-s", "--format=%an%n%ae%n%cn%n%ce%n%s"]).unwrap();
        assert_eq!(first, "Local Developer\ndeveloper@example.invalid\nLocal Developer\ndeveloper@example.invalid\nAdd first result");
        publish_with_summary(
            &f.journal, &second, &f.args, "### Add second result\n\nWrite the parallel output to other.txt.",
        ).unwrap();
        let combined = git(&f.target, &["show", "-s", "--format=%an%n%ae%n%s%n%b"]).unwrap();
        assert!(combined.starts_with("Local Developer\ndeveloper@example.invalid\nAdd second result\n"));
        assert!(combined.contains("Write the parallel output to other.txt."));
        assert_eq!(
            fs::read_to_string(f.target.join("result.txt")).unwrap(),
            "result"
        );
        assert_eq!(
            fs::read_to_string(f.target.join("other.txt")).unwrap(),
            "second result"
        );
        assert_eq!(git(&work, &["rev-parse", "HEAD"]).unwrap(), head);
        assert_eq!(git(&work, &["ls-files", "--stage"]).unwrap(), index);
        let combined = git(&f.target, &["rev-parse", "HEAD"]).unwrap();
        publish(&f.journal, &second, &f.args).unwrap();
        assert_eq!(git(&f.target, &["rev-parse", "HEAD"]).unwrap(), combined);
        second.command.id = CommandId::new();
        fs::write(work.join("other.txt"), "second followup").unwrap();
        publish(&f.journal, &second, &f.args).unwrap();
        assert_eq!(
            fs::read_to_string(f.target.join("result.txt")).unwrap(),
            "result"
        );
        assert_eq!(
            fs::read_to_string(f.target.join("other.txt")).unwrap(),
            "second followup"
        );
        let mut first = f.record.clone();
        first.command.id = CommandId::new();
        fs::write(f.work.join("result.txt"), "first followup").unwrap();
        publish(&f.journal, &first, &f.args).unwrap();
        assert_eq!(
            fs::read_to_string(f.target.join("result.txt")).unwrap(),
            "first followup"
        );
        assert_eq!(
            fs::read_to_string(f.target.join("other.txt")).unwrap(),
            "second followup"
        );
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn automatic_merge_never_executes_repository_merge_drivers() {
        let f = fixture().await;
        let (second, work) = parallel(&f);
        let marker = f._dir.path().join("driver-ran");
        // Concurrent add/add uses the custom driver if it is not overridden.
        fs::write(f.work.join(".gitattributes"), "result.txt merge=hostile\n").unwrap();
        fs::write(work.join(".gitattributes"), "result.txt merge=hostile\n").unwrap();
        fs::write(work.join("result.txt"), "other").unwrap();
        git(
            &work,
            &[
                "config",
                "merge.hostile.driver",
                &format!("touch {}; true", marker.display()),
            ],
        )
        .unwrap();
        publish(&f.journal, &f.record, &f.args).unwrap();
        assert!(publish(&f.journal, &second, &f.args).is_err());
        assert!(!marker.exists());
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn same_file_nonoverlapping_changes_merge_but_delete_modify_conflicts() {
        for deletion in [false, true] {
            let mut f = fixture().await;
            let original = "first\nsecond\nthird\nfourth\nfifth\nsixth\nlast\n";
            fs::write(f.work.join("result.txt"), original).unwrap();
            publish(&f.journal, &f.record, &f.args).unwrap();
            let (second, work) = parallel(&f);
            fs::write(
                work.join("result.txt"),
                original.replace("last", "last changed"),
            )
            .unwrap();
            f.record.command.id = CommandId::new();
            if deletion {
                fs::remove_file(f.work.join("result.txt")).unwrap();
            } else {
                fs::write(
                    f.work.join("result.txt"),
                    original.replace("first", "first changed"),
                )
                .unwrap();
            }
            publish(&f.journal, &f.record, &f.args).unwrap();
            if deletion {
                let error = publish(&f.journal, &second, &f.args).unwrap_err();
                assert!(error.downcast_ref::<PublicationConflict>().is_some());
                assert!(error.to_string().contains("result.txt"));
                assert!(!f.target.join("result.txt").exists());
            } else {
                publish(&f.journal, &second, &f.args).unwrap();
                assert_eq!(
                    fs::read_to_string(f.target.join("result.txt")).unwrap(),
                    original
                        .replace("first", "first changed")
                        .replace("last", "last changed")
                );
            }
        }
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn merger_publishes_both_parents_from_a_separate_worktree() {
        let f = fixture().await;
        let (source, original_work) = parallel(&f);
        fs::write(original_work.join("result.txt"), "competing result").unwrap();
        publish(&f.journal, &f.record, &f.args).unwrap();
        let target = git(&f.target, &["rev-parse", "HEAD"]).unwrap();
        fs::write(f.target.join(".gitignore"), "target/\n# local staged edit\n").unwrap();
        git(&f.target, &["add", "--", ".gitignore"]).unwrap();
        fs::write(f.target.join(".gitignore"), "target/\n# local unstaged edit\n").unwrap();
        fs::write(f.target.join(" local notes"), "untracked user file").unwrap();
        fs::write(f.target.join("result.txt"), "unpublished user changes").unwrap();
        let staged = git(&f.target, &["diff", "--cached", "--", ".gitignore"]).unwrap();
        assert!(publish(&f.journal, &source, &f.args).unwrap_err().is::<PublicationConflict>());
        let mut merger = f.journal.receive(merger_command(&source.command)).unwrap();
        let workspace = execution_workspace(&f.journal, &merger, &f.args, true).unwrap();
        let work = Path::new(&workspace.path);
        assert_ne!(work, original_work);
        assert!(publish(&f.journal, &merger, &f.args).unwrap_err().to_string().contains("both original results"));
        assert_eq!(git(&f.target, &["rev-parse", "HEAD"]).unwrap(), target);
        assert!(managed_commit(work, &["merge", "--no-edit", &target]).is_err());
        fs::write(work.join("result.txt"), "combined result").unwrap();
        git(work, &["add", "--", "result.txt"]).unwrap();
        managed_commit(work, &["commit", "-m", "resolve conflict"]).unwrap();
        let error = publish(&f.journal, &merger, &f.args).unwrap_err();
        assert!(error.to_string().contains("überschneiden"), "{error}");
        assert_eq!(git(&f.target, &["rev-parse", "HEAD"]).unwrap(), target);
        assert_eq!(fs::read_to_string(f.target.join("result.txt")).unwrap(), "unpublished user changes");
        assert_eq!(git(&f.target, &["diff", "--cached", "--", ".gitignore"]).unwrap(), staged);
        assert!(published_commit(&f.journal, merger.command.id).is_err());
        merger.result = Some(CommandOutput::Agent { agent: AgentProgress {
            session_id: merger.command.session_id(), message_id: agent_message_id(merger.command.id),
            phase: AgentPhase::Succeeded, summary: format!("{PUBLICATION_BLOCKED_PREFIX}\n{error}"),
        }});
        merger.reported = true;
        f.journal.save(&merger).unwrap();
        assert_eq!(recovery_candidate(&f.journal, 0).unwrap(), Some(merger.command.id));
        assert!(recover_prepared(&f.journal, merger.command.id, RequestId::new()).is_err());
        assert!(recover_prepared(&f.journal, merger.command.id, RequestId::new()).unwrap().is_none());
        let journal_path = f.args.journal.as_ref().unwrap().clone();
        drop(f.journal);
        let journal = Journal::open(&journal_path, &f.args.server, f.args.id).unwrap();
        assert_eq!(recovery_candidate(&journal, 0).unwrap(), Some(merger.command.id));
        assert!(recover_prepared(&journal, merger.command.id, RequestId::new()).unwrap().is_none());
        fs::write(work.join("result.txt"), "later edit in merger worktree").unwrap();
        // Simulate the user moving the overlapping edit out of the checkout.
        git(&f.target, &["restore", "--", "result.txt"]).unwrap();
        let proof = recover_prepared(&journal, merger.command.id, RequestId::new()).unwrap().unwrap();
        let replay = recover_prepared(&journal, merger.command.id, RequestId::new()).unwrap().unwrap();
        assert_eq!(proof.target_commit, replay.target_commit);
        assert_eq!(proof.snapshot_commit, replay.snapshot_commit);
        acknowledge_recovery(&journal, merger.command.id).unwrap();
        assert!(recovery_candidate(&journal, 0).unwrap().is_none());
        let head = git(&f.target, &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(proof.target_commit, head);
        for commit in [&target, &workspace.base_commit] {
            git(&f.target, &["merge-base", "--is-ancestor", commit, &head]).unwrap();
        }
        assert_eq!(fs::read_to_string(f.target.join("result.txt")).unwrap(), "combined result");
        assert_eq!(fs::read_to_string(original_work.join("result.txt")).unwrap(), "competing result");
        assert_eq!(fs::read_to_string(work.join("result.txt")).unwrap(), "later edit in merger worktree");
        assert_eq!(git(&f.target, &["diff", "--cached", "--", ".gitignore"]).unwrap(), staged);
        assert_eq!(fs::read_to_string(f.target.join(".gitignore")).unwrap(), "target/\n# local unstaged edit\n");
        assert_eq!(fs::read_to_string(f.target.join(" local notes")).unwrap(), "untracked user file");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn parallel_conflict_retains_snapshot_and_explicit_followup_can_resolve_it() {
        let f = fixture().await;
        let (mut second, work) = parallel(&f);
        fs::write(work.join("result.txt"), "competing result").unwrap();
        publish(&f.journal, &f.record, &f.args).unwrap();
        let target = git(&f.target, &["rev-parse", "HEAD"]).unwrap();
        let error = publish(&f.journal, &second, &f.args)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Paralleler Ergebniskonflikt") && error.contains("result.txt"),
            "{error}"
        );
        assert_eq!(git(&f.target, &["rev-parse", "HEAD"]).unwrap(), target);
        assert_eq!(
            fs::read_to_string(work.join("result.txt")).unwrap(),
            "competing result"
        );
        // Restart cannot silently replace the original candidate with later edits.
        fs::write(work.join("result.txt"), "later edits").unwrap();
        let args = f.args.clone();
        let journal_path = args.journal.as_ref().unwrap().clone();
        drop(f.journal);
        let journal = Journal::open(&journal_path, &args.server, args.id).unwrap();
        assert!(publish(&journal, &second, &args)
            .unwrap_err()
            .to_string()
            .contains("Paralleler Ergebniskonflikt"));
        // Targeted follow-up combines both histories in its own execution branch.
        git(&work, &["add", "--all"]).unwrap();
        git(
            &work,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "retain second result",
            ],
        )
        .unwrap();
        assert!(git(
            &work,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "merge",
                "--no-edit",
                &target
            ]
        )
        .is_err());
        fs::write(work.join("result.txt"), "result plus competing result").unwrap();
        git(&work, &["add", "--all"]).unwrap();
        git(
            &work,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "resolve both results",
            ],
        )
        .unwrap();
        second.command.id = CommandId::new();
        publish(&journal, &second, &args).unwrap();
        assert_eq!(
            fs::read_to_string(f.target.join("result.txt")).unwrap(),
            "result plus competing result"
        );
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn publishes_ignored_aware_snapshot_and_next_task_sees_result_without_mutating_execution_index(
    ) {
        let f = fixture().await;
        let head = git(&f.work, &["rev-parse", "HEAD"]).unwrap();
        let index = git(&f.work, &["ls-files", "--stage"]).unwrap();
        assert_eq!(
            publish(&f.journal, &f.record, &f.args).unwrap(),
            Some(f.target.canonicalize().unwrap())
        );
        assert_eq!(
            fs::read_to_string(f.target.join("result.txt")).unwrap(),
            "result"
        );
        assert!(!f.target.join("target").exists());
        assert_eq!(git(&f.work, &["rev-parse", "HEAD"]).unwrap(), head);
        assert_eq!(git(&f.work, &["ls-files", "--stage"]).unwrap(), index);
        let target_head = git(&f.target, &["rev-parse", "HEAD"]).unwrap();
        publish(&f.journal, &f.record, &f.args).unwrap();
        assert_eq!(git(&f.target, &["rev-parse", "HEAD"]).unwrap(), target_head);
        let mut next = f.record.command.clone();
        next.id = CommandId::new();
        next.stream_id = StreamId::new();
        next.workspace_spec.as_mut().unwrap().branch = format!("factory/{}", next.stream_id);
        let CommandOutput::Workspace { workspace } = workspace::prepare(
            &f.journal,
            &next,
            None,
            f.args.workspace_root.as_deref().unwrap(),
        ) else {
            panic!()
        };
        assert_eq!(
            fs::read_to_string(Path::new(&workspace.path).join("result.txt")).unwrap(),
            "result"
        );
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn target_edits_block_publication_and_completed_replay_preserves_later_user_edits() {
        let f = fixture().await;
        fs::write(f.target.join("result.txt"), "user").unwrap();
        assert!(publish(&f.journal, &f.record, &f.args).is_err());
        assert_eq!(
            fs::read_to_string(f.target.join("result.txt")).unwrap(),
            "user"
        );
        fs::remove_file(f.target.join("result.txt")).unwrap();
        publish(&f.journal, &f.record, &f.args).unwrap();
        fs::write(f.target.join("result.txt"), "later user edit").unwrap();
        publish(&f.journal, &f.record, &f.args).unwrap();
        assert_eq!(
            fs::read_to_string(f.target.join("result.txt")).unwrap(),
            "later user edit"
        );
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn refuses_to_overwrite_ignored_target_files_when_result_starts_tracking_them() {
        let mut f = fixture().await;
        fs::write(f.work.join(".gitignore"), "target/\nprivate.txt\n").unwrap();
        publish(&f.journal, &f.record, &f.args).unwrap();
        fs::write(f.target.join("private.txt"), "user data").unwrap();
        fs::write(f.work.join(".gitignore"), "target/\n").unwrap();
        fs::write(f.work.join("private.txt"), "agent output").unwrap();
        f.record.command.id = CommandId::new();
        assert!(publish(&f.journal, &f.record, &f.args).is_err());
        assert_eq!(
            fs::read_to_string(f.target.join("private.txt")).unwrap(),
            "user data"
        );
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn resumes_pinned_snapshot_after_target_fast_forward_without_resnapshotting_new_edits() {
        let f = fixture().await;
        publish(&f.journal, &f.record, &f.args).unwrap();
        let raw: String = f
            .journal
            .db
            .query_row(
                "SELECT data FROM publications WHERE command_id=?1",
                [f.record.command.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        let mut plan: Plan = serde_json::from_str(&raw).unwrap();
        plan.published = false;
        save(&f.journal, f.record.command.id, &plan).unwrap();
        git(
            &plan.repository,
            &["update-ref", "refs/heads/main", &plan.parent, &plan.commit],
        )
        .unwrap();
        fs::write(f.work.join("result.txt"), "later agent edit").unwrap();
        let args = f.args.clone();
        let record = f.record.clone();
        let target = f.target.clone();
        let path = args.journal.as_ref().unwrap().clone();
        drop(f.journal);
        let journal = Journal::open(&path, &args.server, args.id).unwrap();
        publish(&journal, &record, &args).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("result.txt")).unwrap(),
            "result"
        );
        assert_eq!(
            git(&plan.repository, &["rev-parse", "main"]).unwrap(),
            plan.commit
        );
        let mut continuation = record;
        continuation.command.id = CommandId::new();
        publish(&journal, &continuation, &args).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("result.txt")).unwrap(),
            "later agent edit"
        );
    }
}
