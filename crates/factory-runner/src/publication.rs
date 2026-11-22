//! Publish a successful local task via an immutable snapshot and guarded fast-forward.
//! The execution worktree and its index are never modified.
use crate::{
    journal::{Journal, Record},
    workspace::{self, git, git_env},
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
fn unchanged_target(target: &Path, branch: &str, parent: &str) -> Result<()> {
    ensure!(
        git(target, &["symbolic-ref", "HEAD"])? == format!("refs/heads/{branch}"),
        "Projektordner steht auf einem anderen Branch; nichts ueberschrieben"
    );
    ensure!(
        git(target, &["rev-parse", "HEAD"])? == parent,
        "Projektstand wurde inzwischen geaendert; Ergebnis bleibt im Arbeitsbereich"
    );
    ensure!(
        git(target, &["status", "--porcelain", "--untracked-files=all"])?.is_empty(),
        "Projektordner enthaelt eigene Aenderungen; nichts ueberschrieben"
    );
    Ok(())
}
pub fn publish(j: &Journal, r: &Record, args: &Args) -> Result<Option<PathBuf>> {
    let spec = r
        .command
        .workspace_spec
        .as_ref()
        .context("Workspace specification missing")?;
    let Some(setup) = &spec.setup else {
        return Ok(None);
    };
    // Remote URLs and Factory-managed storage have no user-selected local output folder.
    let source = Path::new(&setup.repository_source);
    if !source.is_absolute() || !setup.repository_path.is_empty() {
        return Ok(None);
    }
    let root = args
        .workspace_root
        .as_deref()
        .context("Workspace root missing")?;
    let workspace = workspace::existing(j, &r.command, args.repositories.as_deref(), root)?;
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
        let parent = previous.unwrap_or_else(|| workspace.base_commit.clone());
        let current = git(&target, &["rev-parse", "HEAD"])?;
        unchanged_target(&target, &spec.base_branch, &current)?;
        let work = Path::new(&workspace.path);
        ensure!(
            git(work, &["ls-files", "--unmerged"])?.is_empty(),
            "Arbeitsbereich enthaelt ungeloeste Konflikte"
        );
        let temp = tempfile::tempdir_in(root)?;
        let index = temp.path().join("index");
        let index = index.to_str().context("Non-UTF8 index path")?;
        let env = [("GIT_INDEX_FILE", index)];
        git_env(work, &["read-tree", "HEAD"], &env)?;
        git_env(work, &["add", "--all", "--", "."], &env)?;
        ensure!(
            !git_env(work, &["ls-files", "--stage"], &env)?
                .lines()
                .any(|line| line.starts_with("160000 ")),
            "Verschachtelte Repositories/Submodule werden nicht automatisch uebernommen"
        );
        let tree = git_env(work, &["write-tree"], &env)?;
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
        let message = format!("YoloForge task {}", r.command.id);
        commit_args.extend(["-m", message.as_str()]);
        let commit = git_env(
            work,
            &commit_args,
            &[
                ("GIT_AUTHOR_NAME", "YoloForge"),
                ("GIT_AUTHOR_EMAIL", "factory@yoloforge.invalid"),
                ("GIT_COMMITTER_NAME", "YoloForge"),
                ("GIT_COMMITTER_EMAIL", "factory@yoloforge.invalid"),
            ],
        )?;
        let plan = Plan {
            snapshot: Some(commit.clone()),
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
        let current = git(&target, &["rev-parse", "HEAD"])?;
        if current != plan.parent && current != plan.commit {
            ensure!(
                plan.commit == *snapshot,
                "Projektstand nach gepinnter Zusammenfuehrung geaendert; Ergebnis erhalten"
            );
            unchanged_target(&target, &plan.branch, &current)?;
            git(
                &repository,
                &[
                    "fetch",
                    "--refmap=",
                    "--no-tags",
                    "--no-write-fetch-head",
                    "--no-auto-maintenance",
                    target.to_str().context("Non-UTF8 target")?,
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
            let commit = git_env(
                &repository,
                &[
                    "commit-tree",
                    tree,
                    "-p",
                    &current,
                    "-p",
                    snapshot,
                    "-m",
                    &format!("YoloForge combined task {}", r.command.id),
                ],
                &[
                    ("GIT_AUTHOR_NAME", "YoloForge"),
                    ("GIT_AUTHOR_EMAIL", "factory@yoloforge.invalid"),
                    ("GIT_COMMITTER_NAME", "YoloForge"),
                    ("GIT_COMMITTER_EMAIL", "factory@yoloforge.invalid"),
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
    let target_head = git(&target, &["rev-parse", "HEAD"])?;
    if target_head != plan.commit {
        unchanged_target(&target, &plan.branch, &plan.parent)?;
        git(
            &target,
            &[
                "fetch",
                "--no-tags",
                "--refmap=",
                "--no-write-fetch-head",
                "--no-auto-maintenance",
                repository.to_str().context("Non-UTF8 repository path")?,
                &reference,
            ],
        )?;
        // Recheck after fetch. Git itself refuses overlapping dirty/untracked files at checkout.
        unchanged_target(&target, &plan.branch, &plan.parent)?;
        git(
            &target,
            &[
                "-c",
                "merge.autoStash=false",
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
    plan.published = true;
    save(j, r.command.id, &plan)?;
    Ok(Some(target))
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
    #[tokio::test(flavor = "multi_thread")]
    async fn parallel_disjoint_results_combine_and_continuations_do_not_erase_other_work() {
        let f = fixture().await;
        let (mut second, work) = parallel(&f);
        fs::write(work.join("other.txt"), "second result").unwrap();
        let head = git(&work, &["rev-parse", "HEAD"]).unwrap();
        let index = git(&work, &["ls-files", "--stage"]).unwrap();
        publish(&f.journal, &f.record, &f.args).unwrap();
        publish(&f.journal, &second, &f.args).unwrap();
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
            Some(f.target.clone())
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
