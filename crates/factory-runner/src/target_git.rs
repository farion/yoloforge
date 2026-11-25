//! Execute only the typed plan against the user-selected local project checkout.
use crate::{
    journal::{Journal, Record},
    publication,
    workspace::{git, git_network},
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

/// Write-ahead receipt: retries keep the exact commit, tag and destination.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Execution {
    spec: WorkspaceSpec,
    path: std::path::PathBuf,
    before: String,
    commit: Option<String>,
    tag: Option<String>,
    push: Option<(String, String)>,
    push_url: Option<String>,
    expected: Option<String>,
}

fn version(tag: &str) -> Option<([u64; 3], bool)> {
    let bare = tag.strip_prefix('v').unwrap_or(tag);
    let mut fields = bare.split('.');
    let mut parts = [0; 3];
    for part in &mut parts {
        let field = fields.next()?;
        if !field.bytes().all(|b| b.is_ascii_digit()) || (field.len() > 1 && field.starts_with('0'))
        {
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
    parts[field] = parts[field]
        .checked_add(1)
        .context("Versionsnummer zu groß")?;
    parts[field + 1..].fill(0);
    let [major, minor, patch] = parts;
    Ok(format!(
        "{}{major}.{minor}.{patch}",
        if prefix { "v" } else { "" }
    ))
}

fn target_head(path: &Path, branch: &str) -> Result<String> {
    ensure!(
        git(path, &["symbolic-ref", "HEAD"])? == branch,
        "Zielbranch ist nicht ausgecheckt"
    );
    git(path, &["rev-parse", "--verify", "HEAD"])
}

fn push_destination(
    path: &Path,
    spec: &WorkspaceSpec,
    plan: &TargetGitPlan,
    branch: &str,
) -> Result<(String, String)> {
    let (remote, dest) = match &plan.remote {
        Some(remote) => (remote.clone(), branch.to_string()),
        None => {
            let setting = |key| {
                git(
                    path,
                    &[
                        "config",
                        "--get-all",
                        &format!("branch.{}.{key}", spec.base_branch),
                    ],
                )
                .context("Kein eindeutiger Upstream für den Zielbranch konfiguriert")
            };
            (setting("remote")?, setting("merge")?)
        }
    };
    ensure!(
        remote != "." && git(path, &["remote"])?.lines().any(|name| name == remote),
        "Kein eindeutiger konfigurierter Push-Remote"
    );
    ensure!(dest.starts_with("refs/heads/"), "Upstream ist kein Branch");
    git(path, &["check-ref-format", &dest]).context("Ungültiger Push-Zielbranch")?;
    let urls = git(
        path,
        &["remote", "get-url", "--push", "--all", "--", &remote],
    )?;
    ensure!(
        !urls.is_empty() && urls.lines().count() == 1,
        "Remote hat kein eindeutiges Push-Ziel"
    );
    let mirror = git(
        path,
        &[
            "config",
            "--type=bool",
            "--default=false",
            "--get",
            &format!("remote.{remote}.mirror"),
        ],
    )?;
    ensure!(
        mirror == "false",
        "Mirror-Remotes sind für geplante Pushes nicht zulässig"
    );
    Ok((remote, dest))
}

#[cfg(test)]
fn execute_inner(
    spec: &WorkspaceSpec,
    expected: Option<&str>,
    steps: &mut Vec<String>,
) -> Result<()> {
    execute_saved(spec, expected, steps, &mut None, |_| Ok(()))
}

fn execute_saved(
    spec: &WorkspaceSpec,
    expected: Option<&str>,
    steps: &mut Vec<String>,
    saved: &mut Option<Execution>,
    mut persist: impl FnMut(&Execution) -> Result<()>,
) -> Result<()> {
    let plan = spec
        .target_git
        .as_ref()
        .filter(|p| p.valid())
        .context("Ungültiger Git-Plan")?;
    let setup = spec
        .setup
        .as_ref()
        .filter(|s| s.has_local_target())
        .context("Lokaler Ziel-Checkout fehlt")?;
    let path = fs::canonicalize(&setup.repository_source).context("Projektordner fehlt")?;
    ensure!(
        fs::canonicalize(git(&path, &["rev-parse", "--show-toplevel"])?)
            .ok()
            .as_ref()
            == Some(&path),
        "Projektordner ist nicht die Wurzel eines Git-Checkouts"
    );
    let branch = format!("refs/heads/{}", spec.base_branch);
    git(&path, &["check-ref-format", &branch])?;
    let current = target_head(&path, &branch)?;
    if let Some(previous) = saved.as_ref() {
        ensure!(
            previous.spec == *spec
                && previous.path == path
                && previous.expected.as_deref() == expected,
            "Git-Recovery gehört zu einer anderen Projektkonfiguration"
        );
        ensure!(
            current == previous.before || previous.commit.as_deref() == Some(current.as_str()),
            "Zielbranch wurde seit dem Git-Auftrag verändert; keine automatische Wiederholung"
        );
        if let Some((remote, _)) = &previous.push {
            ensure!(
                Some(push_destination(&path, spec, plan, &branch)?) == previous.push
                    && Some(git(
                        &path,
                        &["remote", "get-url", "--push", "--all", "--", remote]
                    )?) == previous.push_url,
                "Push-Ziel wurde seit dem Git-Auftrag verändert; keine automatische Wiederholung"
            );
        }
    } else {
        if let Some(expected) = expected {
            ensure!(
                current == expected,
                "Zielbranch wurde seit der Übernahme verändert"
            );
        }
        // Check the push destination and tag before making staging/ref changes.
        let push = plan
            .push
            .then(|| push_destination(&path, spec, plan, &branch))
            .transpose()?;
        let tag = if let Some(bump) = &plan.tag {
            let tags = git(&path, &["tag", "--merged", &current, "--list"])?;
            let tag = format!("refs/tags/{}", next_tag(&tags, bump)?);
            git(&path, &["check-ref-format", &tag]).context("Ungültiger Tag-Name")?;
            ensure!(
                !git(&path, &["for-each-ref", "--format=%(refname)", &tag])?
                    .lines()
                    .any(|r| r == tag),
                "Tag existiert bereits und wird nicht überschrieben: {tag}"
            );
            Some(tag)
        } else {
            None
        };
        let initial = Execution {
            spec: spec.clone(),
            path: path.clone(),
            before: current.clone(),
            push_url: push
                .as_ref()
                .map(|(remote, _)| {
                    git(
                        &path,
                        &["remote", "get-url", "--push", "--all", "--", remote],
                    )
                })
                .transpose()?,
            commit: None,
            tag,
            push,
            expected: expected.map(str::to_string),
        };
        persist(&initial)?;
        *saved = Some(initial);
    }
    let receipt = saved.as_mut().context("Git-Ausführungsjournal fehlt")?;
    if receipt.commit.is_none() && plan.commit && expected.is_none() {
        for marker in [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "rebase-merge",
            "rebase-apply",
            "sequencer",
        ] {
            let marker = git(
                &path,
                &["rev-parse", "--path-format=absolute", "--git-path", marker],
            )?;
            ensure!(
                !Path::new(&marker).try_exists()?,
                "Im Ziel-Checkout läuft bereits eine Git-Operation"
            );
        }
        ensure!(
            git(&path, &["ls-files", "--unmerged"])?.is_empty(),
            "Im Ziel-Checkout bestehen ungelöste Git-Konflikte"
        );
        // A direct commit request covers the current project checkout, not
        // only an index that the user would have to prepare manually. Respect
        // ignore rules; never force-add generated files or private data.
        git(&path, &["add", "--all", "--", "."])
            .context("Änderungen im Ziel-Checkout konnten nicht gestagt werden")?;
        let tree = git(&path, &["write-tree"]).context("Änderungen sind nicht commitbar")?;
        if tree
            != git(
                &path,
                &["rev-parse", &format!("{}^{{tree}}", receipt.before)],
            )?
        {
            let message = plan
                .commit_message
                .as_deref()
                .unwrap_or("Commit project changes");
            receipt.commit = Some(publication::commit_tree(
                &path,
                &path,
                &["commit-tree", &tree, "-p", &receipt.before, "-m", message],
            )?);
        } else {
            receipt.commit = Some(receipt.before.clone());
        }
        // Persist the chosen object before updating the branch. A crash after
        // update-ref can then be reconciled without making a second commit.
        persist(receipt)?;
    }
    if receipt.commit.is_none() {
        receipt.commit = Some(receipt.before.clone());
        persist(receipt)?;
    }
    let head = receipt.commit.as_ref().context("Git-Commit fehlt")?;
    let current = target_head(&path, &branch)?;
    if current != *head {
        ensure!(
            current == receipt.before,
            "Zielbranch wurde inzwischen verändert"
        );
        git(
            &path,
            &[
                "update-ref",
                "-m",
                "YoloForge: commit project changes",
                &branch,
                head,
                &receipt.before,
            ],
        )?;
    }
    steps.push(
        if plan.commit && expected.is_none() && *head == receipt.before {
            format!("Keine offenen Änderungen; vorhandener Commit: {head}")
        } else {
            format!("Commit: {head}")
        },
    );
    if let Some(tag) = &receipt.tag {
        ensure!(
            target_head(&path, &branch)? == *head,
            "Zielbranch wurde inzwischen verändert"
        );
        let exists = git(&path, &["for-each-ref", "--format=%(objectname)", tag])?;
        if exists.is_empty() {
            git(&path, &["update-ref", tag, head, &"0".repeat(head.len())])
                .context("Tag konnte nicht sicher angelegt werden")?;
        } else {
            ensure!(
                exists == *head,
                "Geplanter Tag zeigt auf einen anderen Commit; kein Überschreiben"
            );
        }
        steps.push(format!("Tag: {}", tag.trim_start_matches("refs/tags/")));
    }
    if let Some((remote, dest)) = &receipt.push {
        ensure!(
            target_head(&path, &branch)? == *head,
            "Zielbranch wurde inzwischen verändert"
        );
        let branch_spec = format!("{head}:{dest}");
        let mut args = vec![
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.ssh.allow=always",
            "-c",
            "protocol.https.allow=always",
            "push",
            "--no-follow-tags",
            "--no-mirror",
            "--no-force",
            "--no-prune",
            "--no-push-option",
            "--recurse-submodules=no",
            "--no-verify",
        ];
        let tag_spec;
        if receipt.tag.is_some() {
            args.push("--atomic");
        }
        args.extend(["--", &remote, &branch_spec]);
        if let Some(tag) = &receipt.tag {
            tag_spec = format!("{head}:{tag}");
            args.push(&tag_spec);
        }
        git_network(&path, &args)
            .context("Push nicht bestätigt; Remote prüfen, bevor erneut veröffentlicht wird")?;
        steps.push(format!("Push: {remote}/{dest}"));
    }
    Ok(())
}

pub fn execute(journal: &Journal, record: &mut Record, expected: Option<&str>) -> (bool, String) {
    let mut steps = vec![];
    let result = (|| {
        ensure!(
            !record.recovery_required,
            "Runner-Journal fehlt; Git-Ergebnis vor Wiederholung prüfen"
        );
        let mut spec = record
            .command
            .workspace_spec
            .clone()
            .context("Ziel-Checkout fehlt")?;
        let resuming = spec.target_git.as_ref().is_some_and(|p| p.resume);
        if let Some(plan) = &mut spec.target_git {
            plan.resume = false;
        }
        // A direct Git retry stays on the same stream/owner. Mixed tasks keep
        // separate receipts for each publication command.
        let key = if record.command.kind == CommandKind::TargetGit {
            format!(
                "stream/{}/{}",
                record.command.stream_id, record.command.ownership_epoch
            )
        } else {
            format!("command/{}", record.command.id)
        };
        let raw: Option<String> = journal
            .db
            .query_row(
                "SELECT data FROM target_git_executions WHERE id=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()?;
        let mut saved: Option<Execution> = raw.map(|s| serde_json::from_str(&s)).transpose()?;
        if saved.is_none() {
            ensure!(!record.target_git_started,
                "Git-Schritte ohne Ausführungsjournal unterbrochen; Ziel-Checkout und Remote prüfen");
            let mut proven_preflight = false;
            let mut stmt = journal.db.prepare(
                "SELECT data FROM commands WHERE id<>?1
                AND json_extract(data,'$.command.stream_id')=?2
                AND json_extract(data,'$.command.ownership_epoch')=?3
                AND json_extract(data,'$.command.kind')='target_git'",
            )?;
            for raw in stmt.query_map(
                params![
                    record.command.id.to_string(),
                    record.command.stream_id.to_string(),
                    record.command.ownership_epoch
                ],
                |r| r.get::<_, String>(0),
            )? {
                let raw = raw?;
                let metadata: serde_json::Value = serde_json::from_str(&raw)?;
                let explicitly_unstarted = metadata["target_git_started"].as_bool() == Some(false);
                let previous: Record = serde_json::from_str(&raw)?;
                // The original v1 implementation emitted this exact pre-commit
                // failure before any ref/push mutation. All other legacy
                // outcomes lack sufficient evidence for automatic replay.
                let preflight_only = matches!(&previous.result, Some(CommandOutput::TargetGit { git_succeeded: false, git_summary })
                    if explicitly_unstarted || git_summary == "Git-Schritte angehalten: Keine gestagten Änderungen zum Committen");
                let mut previous_spec = previous.command.workspace_spec.clone();
                if let Some(plan) = previous_spec.as_mut().and_then(|s| s.target_git.as_mut()) {
                    plan.resume = false;
                }
                ensure!(preflight_only && !previous.recovery_required && previous_spec == Some(spec.clone()),
                    "Früherer Git-Auftrag ohne Ausführungsjournal; Ergebnis vor Wiederholung prüfen");
                proven_preflight = true;
            }
            ensure!(!resuming || proven_preflight, "Git-Ausführungsjournal für die Wiederholung fehlt; Commit, Tag und Remote zuerst prüfen");
        }
        execute_saved(&spec, expected, &mut steps, &mut saved, |state| {
            journal.db.execute(
                "INSERT INTO target_git_executions(id,data) VALUES(?1,?2)
                ON CONFLICT(id) DO UPDATE SET data=excluded.data",
                params![key, serde_json::to_string(state)?],
            )?;
            record.target_git_started = true;
            journal.save(record)?;
            Ok(())
        })
    })();
    if let Err(error) = &result {
        steps.push(format!("Git-Schritte angehalten: {error}"));
    }
    let mut summary = steps.join("; ");
    let mut end = summary.len().min(1024);
    while !summary.is_char_boundary(end) {
        end -= 1;
    }
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
    fn direct_plan_stages_project_changes_and_tags_the_target_commit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        git(path, &["init", "-b", "main"]).unwrap();
        git(path, &["config", "user.name", "Test"]).unwrap();
        git(path, &["config", "user.email", "test@example.invalid"]).unwrap();
        git(path, &["commit", "--allow-empty", "-m", "Initial"]).unwrap();
        git(path, &["tag", "v2.3.4"]).unwrap();
        fs::write(path.join(".gitignore"), "ignored\n").unwrap();
        fs::write(path.join("ignored"), "generated data").unwrap();
        fs::write(path.join("result"), "staged").unwrap();
        git(path, &["add", "result"]).unwrap();
        fs::write(path.join("result"), "unstaged").unwrap();
        fs::write(path.join("private"), "untracked").unwrap();
        let mut spec = WorkspaceSpec {
            target_git: Some(TargetGitPlan {
                resume: false,
                commit: true,
                tag: Some(TargetTag::Patch),
                push: true,
                commit_message: Some("Requested message".into()),
                remote: None,
            }),
            setup: Some(ProjectSetup {
                repository_source: path.display().to_string(),
                runner_id: RunnerId::new(),
                repository_path: String::new(),
                service_registration: String::new(),
                harness: None,
            }),
            checkpoint: None,
            project_id: ProjectId::new(),
            branch: "factory/unused".into(),
            base_branch: "main".into(),
        };
        let before = git(path, &["rev-parse", "HEAD"]).unwrap();
        let index_before = git(path, &["write-tree"]).unwrap();
        let mut steps = vec![];
        assert!(execute_inner(&spec, None, &mut steps)
            .unwrap_err()
            .to_string()
            .contains("Upstream"));
        assert_eq!(git(path, &["rev-parse", "HEAD"]).unwrap(), before);
        assert_eq!(
            git(path, &["write-tree"]).unwrap(),
            index_before,
            "Invalid push configuration must not stage any files"
        );
        assert_eq!(git(path, &["tag", "--list"]).unwrap(), "v2.3.4");

        git(
            path,
            &[
                "remote",
                "add",
                "upstream",
                "https://example.invalid/project",
            ],
        )
        .unwrap();
        git(path, &["config", "branch.main.remote", "upstream"]).unwrap();
        git(path, &["config", "branch.main.merge", "refs/heads/release"]).unwrap();
        assert_eq!(
            push_destination(
                path,
                &spec,
                spec.target_git.as_ref().unwrap(),
                "refs/heads/main"
            )
            .unwrap(),
            ("upstream".into(), "refs/heads/release".into())
        );
        git(path, &["config", "remote.upstream.mirror", "true"]).unwrap();
        assert!(push_destination(
            path,
            &spec,
            spec.target_git.as_ref().unwrap(),
            "refs/heads/main"
        )
        .is_err());
        spec.target_git.as_mut().unwrap().push = false;
        execute_inner(&spec, None, &mut steps).unwrap();
        assert_eq!(git(path, &["show", "HEAD:result"]).unwrap(), "unstaged");
        assert_eq!(git(path, &["show", "HEAD:private"]).unwrap(), "untracked");
        assert_eq!(
            git(path, &["ls-tree", "--name-only", "HEAD", "--", "ignored"]).unwrap(),
            ""
        );
        assert_eq!(
            git(path, &["log", "-1", "--format=%s"]).unwrap(),
            "Requested message"
        );
        assert_eq!(
            git(path, &["rev-parse", "HEAD"]).unwrap(),
            git(path, &["rev-parse", "v2.3.5"]).unwrap()
        );
        assert_eq!(git(path, &["status", "--porcelain"]).unwrap(), "");
        assert_eq!(git(path, &["branch", "--list", "factory/*"]).unwrap(), "");

        spec.target_git = Some(TargetGitPlan {
            tag: Some(TargetTag::Patch),
            ..Default::default()
        });
        assert!(execute_inner(&spec, Some(&before), &mut vec![]).is_err());
        assert_eq!(git(path, &["tag", "--list"]).unwrap(), "v2.3.4\nv2.3.5");
    }

    #[test]
    fn clean_checkout_still_tags_existing_commits_and_stages_later_deletions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        git(path, &["init", "-b", "main"]).unwrap();
        git(path, &["config", "user.name", "Test"]).unwrap();
        git(path, &["config", "user.email", "test@example.invalid"]).unwrap();
        fs::write(path.join("tracked"), "existing work").unwrap();
        git(path, &["add", "--all"]).unwrap();
        git(path, &["commit", "-m", "Existing local commit"]).unwrap();
        git(path, &["tag", "v1.0.0"]).unwrap();
        let spec = WorkspaceSpec {
            target_git: Some(TargetGitPlan {
                commit: true,
                tag: Some(TargetTag::Patch),
                ..Default::default()
            }),
            setup: Some(ProjectSetup {
                repository_source: path.display().to_string(),
                runner_id: RunnerId::new(),
                repository_path: String::new(),
                service_registration: String::new(),
                harness: None,
            }),
            checkpoint: None,
            project_id: ProjectId::new(),
            branch: "factory/unused".into(),
            base_branch: "main".into(),
        };
        let before = git(path, &["rev-parse", "HEAD"]).unwrap();
        let mut steps = vec![];
        execute_inner(&spec, None, &mut steps).unwrap();
        assert_eq!(git(path, &["rev-parse", "HEAD"]).unwrap(), before);
        assert_eq!(git(path, &["rev-parse", "v1.0.1"]).unwrap(), before);
        assert!(steps
            .iter()
            .any(|s| s.starts_with("Keine offenen Änderungen")));
        fs::remove_file(path.join("tracked")).unwrap();
        execute_inner(&spec, None, &mut vec![]).unwrap();
        assert_eq!(git(path, &["ls-tree", "--name-only", "HEAD"]).unwrap(), "");
        assert_eq!(git(path, &["status", "--porcelain"]).unwrap(), "");
        assert_eq!(
            git(path, &["rev-parse", "v1.0.2"]).unwrap(),
            git(path, &["rev-parse", "HEAD"]).unwrap()
        );
    }

    #[test]
    fn interrupted_git_operation_does_not_stage_uncommitted_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        git(path, &["init", "-b", "main"]).unwrap();
        git(path, &["config", "user.name", "Test"]).unwrap();
        git(path, &["config", "user.email", "test@example.invalid"]).unwrap();
        git(path, &["commit", "--allow-empty", "-m", "Initial"]).unwrap();
        fs::write(path.join("untracked"), "preserve").unwrap();
        fs::write(
            path.join(".git/MERGE_HEAD"),
            git(path, &["rev-parse", "HEAD"]).unwrap(),
        )
        .unwrap();
        let spec = WorkspaceSpec {
            target_git: Some(TargetGitPlan {
                commit: true,
                ..Default::default()
            }),
            setup: Some(ProjectSetup {
                repository_source: path.display().to_string(),
                runner_id: RunnerId::new(),
                repository_path: String::new(),
                service_registration: String::new(),
                harness: None,
            }),
            checkpoint: None,
            project_id: ProjectId::new(),
            branch: "factory/unused".into(),
            base_branch: "main".into(),
        };
        assert!(execute_inner(&spec, None, &mut vec![])
            .unwrap_err()
            .to_string()
            .contains("Git-Operation"));
        assert_eq!(git(path, &["diff", "--cached", "--name-only"]).unwrap(), "");
        assert_eq!(
            fs::read_to_string(path.join("untracked")).unwrap(),
            "preserve"
        );
    }

    fn recovery_fixture(root: &Path, push: bool) -> (Journal, Record, std::path::PathBuf) {
        let path = root.join("project");
        fs::create_dir(&path).unwrap();
        git(&path, &["init", "-b", "main"]).unwrap();
        git(&path, &["config", "user.name", "Test"]).unwrap();
        git(&path, &["config", "user.email", "test@example.invalid"]).unwrap();
        git(&path, &["commit", "--allow-empty", "-m", "Initial"]).unwrap();
        git(&path, &["tag", "v1.0.0"]).unwrap();
        git(
            &path,
            &["remote", "add", "origin", "https://127.0.0.1:1/unreachable"],
        )
        .unwrap();
        git(&path, &["config", "branch.main.remote", "origin"]).unwrap();
        git(&path, &["config", "branch.main.merge", "refs/heads/main"]).unwrap();
        let runner = RunnerId::new();
        let journal =
            Journal::open(&root.join("runner.db"), "http://factory.invalid", runner).unwrap();
        let record = journal
            .receive(RunnerCommand {
                id: CommandId::new(),
                stream_id: StreamId::new(),
                runner_id: runner,
                ownership_epoch: 1,
                kind: CommandKind::TargetGit,
                state: CommandState::Pending,
                result: None,
                agent_spec: None,
                agent_progress: None,
                created_unix_ms: 1,
                workspace_spec: Some(WorkspaceSpec {
                    target_git: Some(TargetGitPlan {
                        commit: true,
                        tag: Some(TargetTag::Patch),
                        push,
                        ..Default::default()
                    }),
                    setup: Some(ProjectSetup {
                        repository_source: path.display().to_string(),
                        runner_id: runner,
                        repository_path: String::new(),
                        service_registration: String::new(),
                        harness: None,
                    }),
                    checkpoint: None,
                    project_id: ProjectId::new(),
                    branch: "factory/unused".into(),
                    base_branch: "main".into(),
                }),
            })
            .unwrap();
        (journal, record, path)
    }

    #[test]
    fn failed_push_retry_after_restart_reuses_commit_and_tag_without_staging_new_work() {
        let dir = tempfile::tempdir().unwrap();
        let (journal, mut record, path) = recovery_fixture(dir.path(), true);
        fs::write(path.join("change"), "requested change").unwrap();
        let (success, summary) = execute(&journal, &mut record, None);
        assert!(!success);
        assert!(summary.contains("Push nicht bestätigt"), "{summary}");
        let commit = git(&path, &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(git(&path, &["rev-parse", "v1.0.1"]).unwrap(), commit);
        record.result = Some(CommandOutput::TargetGit {
            git_succeeded: success,
            git_summary: summary,
        });
        journal.save(&record).unwrap();
        let mut next = record.command.clone();
        next.id = CommandId::new();
        next.created_unix_ms += 1;
        next.workspace_spec
            .as_mut()
            .unwrap()
            .target_git
            .as_mut()
            .unwrap()
            .resume = true;
        drop(journal);
        fs::write(path.join("later-user-work"), "do not include on retry").unwrap();
        let journal = Journal::open(
            &dir.path().join("runner.db"),
            "http://factory.invalid",
            next.runner_id,
        )
        .unwrap();
        let mut retry = journal.receive(next).unwrap();
        let (success, summary) = execute(&journal, &mut retry, None);
        assert!(!success);
        assert!(summary.contains("Push nicht bestätigt"), "{summary}");
        assert_eq!(git(&path, &["rev-parse", "HEAD"]).unwrap(), commit);
        assert_eq!(git(&path, &["tag", "--list"]).unwrap(), "v1.0.0\nv1.0.1");
        assert_eq!(
            git(&path, &["status", "--porcelain"]).unwrap(),
            "?? later-user-work"
        );
        git(
            &path,
            &[
                "remote",
                "set-url",
                "origin",
                "https://127.0.0.1:2/different",
            ],
        )
        .unwrap();
        let (_, summary) = execute(&journal, &mut retry, None);
        assert!(summary.contains("Push-Ziel wurde"), "{summary}");
    }

    #[test]
    fn interrupted_ref_update_reconciles_only_the_pinned_commit_and_refuses_branch_drift() {
        let dir = tempfile::tempdir().unwrap();
        let (_journal, record, path) = recovery_fixture(dir.path(), false);
        fs::write(path.join("change"), "requested change").unwrap();
        let spec = record.command.workspace_spec.as_ref().unwrap();
        let mut saved = None;
        let error = execute_saved(spec, None, &mut vec![], &mut saved, |receipt| {
            if receipt.commit.is_some() {
                anyhow::bail!("Simulated crash after write-ahead receipt");
            }
            Ok(())
        })
        .unwrap_err();
        assert!(error.to_string().contains("Simulated crash"));
        let pinned = saved.as_ref().unwrap().commit.clone().unwrap();
        assert_ne!(git(&path, &["rev-parse", "HEAD"]).unwrap(), pinned);
        execute_saved(spec, None, &mut vec![], &mut saved, |_| Ok(())).unwrap();
        assert_eq!(git(&path, &["rev-parse", "HEAD"]).unwrap(), pinned);
        execute_saved(spec, None, &mut vec![], &mut saved, |_| Ok(())).unwrap();
        assert_eq!(git(&path, &["tag", "--list"]).unwrap(), "v1.0.0\nv1.0.1");
        git(
            &path,
            &["commit", "--allow-empty", "-m", "User changed branch"],
        )
        .unwrap();
        assert!(
            execute_saved(spec, None, &mut vec![], &mut saved, |_| Ok(()))
                .unwrap_err()
                .to_string()
                .contains("Zielbranch wurde")
        );
    }

    #[test]
    fn legacy_no_staged_changes_can_be_retried_but_unknown_partial_git_cannot() {
        for known_preflight in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let (journal, mut old, path) = recovery_fixture(dir.path(), false);
            old.target_git_started = true;
            old.result = Some(CommandOutput::TargetGit {
                git_succeeded: false,
                git_summary: if known_preflight {
                    "Git-Schritte angehalten: Keine gestagten Änderungen zum Committen"
                } else {
                    "Git-Schritte angehalten: Push nicht bestätigt"
                }
                .into(),
            });
            journal.save(&old).unwrap();
            fs::write(path.join("change"), "include on first safe attempt").unwrap();
            let mut command = old.command.clone();
            command.id = CommandId::new();
            command.created_unix_ms += 1;
            command
                .workspace_spec
                .as_mut()
                .unwrap()
                .target_git
                .as_mut()
                .unwrap()
                .resume = true;
            let mut next = journal.receive(command).unwrap();
            let (success, summary) = execute(&journal, &mut next, None);
            assert_eq!(success, known_preflight, "{summary}");
            if known_preflight {
                assert_eq!(
                    git(&path, &["show", "HEAD:change"]).unwrap(),
                    "include on first safe attempt"
                );
            } else {
                assert!(summary.contains("ohne Ausführungsjournal"), "{summary}");
                assert_eq!(git(&path, &["tag", "--list"]).unwrap(), "v1.0.0");
            }
        }
    }

    #[test]
    fn retry_with_missing_journal_never_creates_another_release() {
        let dir = tempfile::tempdir().unwrap();
        let (journal, mut record, path) = recovery_fixture(dir.path(), false);
        // A new pending retry command can arrive after the old journal was lost.
        // Its explicit resume bit must prevent treating it as a fresh release.
        record
            .command
            .workspace_spec
            .as_mut()
            .unwrap()
            .target_git
            .as_mut()
            .unwrap()
            .resume = true;
        fs::write(path.join("user-work"), "preserve").unwrap();
        let (success, summary) = execute(&journal, &mut record, None);
        assert!(!success);
        assert!(summary.contains("Ausführungsjournal"), "{summary}");
        assert_eq!(git(&path, &["tag", "--list"]).unwrap(), "v1.0.0");
        assert_eq!(
            git(&path, &["status", "--porcelain"]).unwrap(),
            "?? user-work"
        );
    }
}
