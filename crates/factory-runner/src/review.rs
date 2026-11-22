//! Review a pinned merge candidate in a disposable runtime; integrate only via Git CAS.
use crate::{
    containment::{daemon, docker, Config},
    journal::{Journal, Record},
    workspace::git,
    Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::os::unix::fs::MetadataExt;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    job: ReviewJob,
    repository: PathBuf,
    work: PathBuf,
    tree: String,
    candidate: String,
    daemon: String,
    image: String,
    socket: PathBuf,
    work_ready: bool,
    #[serde(default)]
    git_pointer: String,
    create_attempted: bool,
    container: Option<String>,
    start_attempted: bool,
    started_at: Option<String>,
    deadline: u64,
    exit: Option<i32>,
    remove_attempted: bool,
    removed: Option<RemovalProof>,
}
impl Plan {
    pub(crate) fn hosting_source(&self) -> Result<(PathBuf, String)> {
        ensure!(
            self.removed.is_some() && self.exit == Some(0),
            "Review runtime must be removed after successful tests"
        );
        let ReviewJob::Review { target_branch, .. } = &self.job else {
            anyhow::bail!("Only a review candidate can be published");
        };
        Ok((self.repository.clone(), target_branch.clone()))
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn save(j: &Journal, r: &mut Record, p: &Plan) -> Result<()> {
    r.review = Some(p.clone());
    tokio::task::block_in_place(|| j.save(r))
}
fn error(text: impl Into<String>) -> Option<CommandOutput> {
    Some(CommandOutput::ReviewError {
        review_error: text.into().chars().take(240).collect(),
    })
}
fn repository(r: &Record, args: &Args) -> Result<PathBuf> {
    let spec = r
        .command
        .workspace_spec
        .as_ref()
        .context("Workspace spec missing")?;
    crate::workspace::repository(
        spec,
        args.repositories.as_deref(),
        args.workspace_root
            .as_deref()
            .context("Workspace root missing")?,
    )
}
fn head(repo: &std::path::Path, branch: &str) -> Result<String> {
    git(
        repo,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{branch}^{{commit}}"),
        ],
    )
}
fn plan(r: &Record, args: &Args, job: &ReviewJob) -> Result<Plan> {
    let repo = repository(r, args)?;
    let root = fs::canonicalize(
        args.workspace_root
            .as_ref()
            .context("Workspace root missing")?,
    )?;
    let work = root.join(".reviews").join(r.command.id.to_string());
    let (tree, candidate) = match job {
        ReviewJob::Review {
            checkpoint,
            target_branch,
            target_commit,
            ..
        } => {
            ensure!(
                head(&repo, target_branch)? == *target_commit,
                "Review target branch changed"
            );
            crate::checkpoint::import_objects(&repo, &root, checkpoint)?;
            let merged = git(
                &repo,
                &[
                    "merge-tree",
                    "--write-tree",
                    target_commit,
                    &checkpoint.commit,
                ],
            )
            .context("Merge conflict or invalid ancestry; resolve in a new checkpoint")?;
            let tree = merged
                .lines()
                .next()
                .context("Merge tree missing")?
                .to_string();
            ensure!(
                matches!(tree.len(), 40 | 64) && tree.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid merge tree"
            );
            let date = format!("@{} +0000", r.command.created_unix_ms / 1000);
            let candidate = crate::workspace::git_env(
                &repo,
                &[
                    "commit-tree",
                    &tree,
                    "-p",
                    target_commit,
                    "-p",
                    &checkpoint.commit,
                    "-m",
                    &format!("YoloForge reviewed integration {}", r.command.id),
                ],
                &[
                    ("GIT_AUTHOR_NAME", "YoloForge"),
                    ("GIT_AUTHOR_EMAIL", "review@yoloforge.invalid"),
                    ("GIT_COMMITTER_NAME", "YoloForge"),
                    ("GIT_COMMITTER_EMAIL", "review@yoloforge.invalid"),
                    ("GIT_AUTHOR_DATE", &date),
                    ("GIT_COMMITTER_DATE", &date),
                ],
            )?;
            ensure!(
                fs::symlink_metadata(&work).is_err(),
                "Unjournaled review worktree exists"
            );
            (tree, candidate)
        }
        ReviewJob::Integrate {
            candidate_commit, ..
        } => (String::new(), candidate_commit.clone()),
    };
    Ok(Plan {
        job: job.clone(),
        repository: repo,
        work,
        tree,
        candidate,
        daemon: String::new(),
        image: String::new(),
        socket: args.review_docker_socket.clone(),
        work_ready: false,
        git_pointer: String::new(),
        create_attempted: false,
        container: None,
        start_attempted: false,
        started_at: None,
        deadline: 0,
        exit: None,
        remove_attempted: false,
        removed: None,
    })
}
fn prepare(p: &mut Plan, r: &Record) -> Result<()> {
    let reference = format!("refs/yoloforge/reviews/{}", r.command.id);
    let previous = git(
        &p.repository,
        &["for-each-ref", "--format=%(objectname)", &reference],
    )?;
    if previous.is_empty() {
        git(
            &p.repository,
            &[
                "update-ref",
                &reference,
                &p.candidate,
                &"0".repeat(p.candidate.len()),
            ],
        )?;
    } else {
        ensure!(previous == p.candidate, "Review ref changed");
    }
    if !p.work.exists() {
        ensure!(
            !p.work_ready && fs::symlink_metadata(&p.work).is_err(),
            "Review worktree lost or replaced"
        );
        fs::create_dir_all(p.work.parent().unwrap())?;
        ensure!(
            !fs::symlink_metadata(p.work.parent().unwrap())?
                .file_type()
                .is_symlink(),
            "Review directory is a symlink"
        );
        git(
            &p.repository,
            &[
                "worktree",
                "add",
                "--detach",
                p.work.to_str().context("Non-UTF8 review path")?,
                &p.candidate,
            ],
        )?;
    }
    ensure!(
        !fs::symlink_metadata(&p.work)?.file_type().is_symlink() && p.work.join(".git").is_file(),
        "Review worktree identity changed"
    );
    ensure!(
        git(&p.work, &["rev-parse", "HEAD"])? == p.candidate
            && git(
                &p.work,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"]
            )? == git(
                &p.repository,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"]
            )?,
        "Review worktree rebound"
    );
    p.work_ready = true;
    ensure!(
        !fs::symlink_metadata(p.work.join(".git"))?
            .file_type()
            .is_symlink(),
        "Review Git pointer is a symlink"
    );
    p.git_pointer = fs::read_to_string(p.work.join(".git"))?;
    Ok(())
}
fn integrate(p: &Plan) -> Result<CommandOutput> {
    let ReviewJob::Integrate {
        review_command,
        target_branch,
        target_commit,
        candidate_commit,
    } = &p.job
    else {
        anyhow::bail!("Not an integration");
    };
    let current = head(&p.repository, target_branch)?;
    if current != *candidate_commit {
        ensure!(
            current == *target_commit,
            "Integration target moved; create and review a fresh checkpoint"
        );
        let occupied = format!("branch refs/heads/{target_branch}");
        ensure!(
            !git(&p.repository, &["worktree", "list", "--porcelain"])?
                .lines()
                .any(|l| l == occupied),
            "Integration target is checked out; use a bare repository or detach its worktree first"
        );
        git(
            &p.repository,
            &[
                "update-ref",
                &format!("refs/heads/{target_branch}"),
                candidate_commit,
                target_commit,
            ],
        )?;
    }
    ensure!(
        head(&p.repository, target_branch)? == *candidate_commit,
        "Integration outcome changed"
    );
    Ok(CommandOutput::Integration {
        integration: IntegrationResult {
            review_command: *review_command,
            target_branch: target_branch.clone(),
            previous_commit: target_commit.clone(),
            commit: candidate_commit.clone(),
        },
    })
}
async fn inspect(c: &Config, id: &str) -> Result<Option<Value>> {
    let ids = docker(
        c,
        &["ps", "-aq", "--no-trunc", "--filter", &format!("id={id}")],
    )
    .await?;
    if ids.is_empty() {
        return Ok(None);
    }
    ensure!(
        std::str::from_utf8(&ids)?.trim() == id,
        "Ambiguous review runtime"
    );
    let values: Vec<Value> =
        serde_json::from_slice(&docker(c, &["inspect", "--type", "container", id]).await?)?;
    ensure!(values.len() == 1, "Ambiguous review inspect");
    Ok(values.into_iter().next())
}
fn validate(v: &Value, p: &Plan, r: &Record) -> Result<()> {
    let common = git(
        &p.repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let mounts = v["Mounts"].as_array().context("Review mounts missing")?;
    ensure!(
        mounts.len() == 2
            && mounts.iter().all(|m| m["Type"] == "bind"
                && m["Source"] == m["Destination"]
                && ((m["Source"] == p.work.to_string_lossy().as_ref() && m["RW"] == true)
                    || (m["Source"] == common && m["RW"] == false))),
        "Review mounts changed"
    );
    ensure!(
        v["HostConfig"]["CapDrop"]
            .as_array()
            .is_some_and(|v| v.iter().any(|v| v == "ALL"))
            && v["HostConfig"]["SecurityOpt"]
                .as_array()
                .is_some_and(|v| v.iter().any(|v| v
                    .as_str()
                    .is_some_and(|s| s.starts_with("no-new-privileges"))))
            && v["Config"]["User"]
                == format!(
                    "{}:{}",
                    fs::metadata(&p.work)?.uid(),
                    fs::metadata(&p.work)?.gid()
                ),
        "Review privilege policy changed"
    );
    ensure!(
        v["Image"] == p.image
            && v["Config"]["Labels"]["io.yoloforge.review"] == r.command.id.to_string()
            && v["HostConfig"]["ReadonlyRootfs"] == true
            && v["HostConfig"]["Privileged"] == false
            && v["HostConfig"]["NetworkMode"] == "none"
            && v["HostConfig"]["RestartPolicy"]["Name"] == "no",
        "Review container configuration changed"
    );
    ensure!(
        v["State"]["Restarting"] == false,
        "Review runtime restarting"
    );
    if let Some(start) = &p.started_at {
        ensure!(
            v["State"]["StartedAt"] == *start,
            "Review runtime restarted; outcome unverified"
        );
    }
    Ok(())
}
/// None means tests still run. Any uncertain live-runtime error keeps the command reserved.
pub async fn step(
    j: &Journal,
    r: &mut Record,
    args: &Args,
    job: &ReviewJob,
) -> Result<Option<CommandOutput>> {
    let mut p = if let Some(p) = &r.review {
        ensure!(
            p.job == *job && p.repository == repository(r, args)?,
            "Review job/mapping changed"
        );
        p.clone()
    } else {
        let p = match tokio::task::block_in_place(|| plan(r, args, job)) {
            Ok(p) => p,
            Err(e) => return Ok(error(e.to_string())),
        };
        save(j, r, &p)?;
        p
    };
    if matches!(job, ReviewJob::Integrate { .. }) {
        return Ok(Some(match tokio::task::block_in_place(|| integrate(&p)) {
            Ok(v) => v,
            Err(e) => CommandOutput::ReviewError {
                review_error: e.to_string().chars().take(240).collect(),
            },
        }));
    }
    if !p.work_ready {
        if let Err(e) = tokio::task::block_in_place(|| prepare(&mut p, r)) {
            return Ok(error(e.to_string()));
        }
        save(j, r, &p)?;
    }
    let socket = fs::canonicalize(&args.review_docker_socket)?;
    let mut c = Config {
        stream_id: r.command.stream_id,
        container_id: String::new(),
        docker_socket: socket.clone(),
    };
    if p.daemon.is_empty() {
        let Some(image) = &args.review_image else {
            return Ok(error(
                "Configure --review-image with a locally installed test image",
            ));
        };
        p.daemon = daemon(&c).await?;
        p.socket = socket;
        let images: Vec<Value> =
            serde_json::from_slice(&docker(&c, &["image", "inspect", image]).await?)?;
        ensure!(images.len() == 1, "Ambiguous review image");
        p.image = images[0]["Id"].as_str().context("Image ID missing")?.into();
        save(j, r, &p)?;
    }
    ensure!(
        p.socket == c.docker_socket && daemon(&c).await? == p.daemon,
        "Review Docker daemon changed"
    );
    let name = format!("yoloforge-review-{}", r.command.id);
    if p.container.is_none() {
        if p.create_attempted {
            let ids = docker(
                &c,
                &[
                    "ps",
                    "-aq",
                    "--no-trunc",
                    "--filter",
                    &format!("name=^/{name}$"),
                ],
            )
            .await?;
            let id = std::str::from_utf8(&ids)?.trim();
            if id.is_empty() {
                return Ok(error("Review create not confirmed; no test was started"));
            }
            ensure!(id.len() == 64, "Ambiguous review creation");
            p.container = Some(id.into());
            save(j, r, &p)?;
        } else {
            let ReviewJob::Review { test_command, .. } = job else {
                unreachable!()
            };
            let uid = fs::metadata(&p.work)?.uid();
            let gid = fs::metadata(&p.work)?.gid();
            ensure!(uid != 0, "Review worktree must be owned by a non-root user");
            let common = git(
                &p.repository,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            )?;
            p.create_attempted = true;
            save(j, r, &p)?;
            let id = docker(
                &c,
                &[
                    "create",
                    "--name",
                    &name,
                    "--read-only",
                    "--user",
                    &format!("{uid}:{gid}"),
                    "--cap-drop",
                    "ALL",
                    "--security-opt",
                    "no-new-privileges",
                    "--network",
                    "none",
                    "--ipc",
                    "private",
                    "--cgroupns",
                    "private",
                    "--restart",
                    "no",
                    "--pids-limit",
                    "256",
                    "--memory",
                    "2g",
                    "--cpus",
                    "2",
                    "--log-opt",
                    "max-size=256k",
                    "--log-opt",
                    "max-file=1",
                    "--tmpfs",
                    "/tmp:rw,nosuid,nodev,size=256m",
                    "--env",
                    "HOME=/tmp",
                    "--label",
                    &format!("io.yoloforge.review={}", r.command.id),
                    "--mount",
                    &format!(
                        "type=bind,source={},target={}",
                        p.work.display(),
                        p.work.display()
                    ),
                    "--mount",
                    &format!("type=bind,source={common},target={common},readonly"),
                    "--workdir",
                    p.work.to_str().context("Non-UTF8 worktree")?,
                    "--entrypoint",
                    "/bin/sh",
                    &p.image,
                    "-lc",
                    test_command,
                ],
            )
            .await?;
            p.container = Some(std::str::from_utf8(&id)?.trim().into());
            save(j, r, &p)?;
        }
    }
    c.container_id = p.container.clone().unwrap();
    let mut observed = inspect(&c, &c.container_id).await?;
    if let Some(v) = &observed {
        validate(v, &p, r)?;
    }
    if !p.start_attempted {
        ensure!(
            observed
                .as_ref()
                .is_some_and(|v| v["State"]["Status"] == "created"),
            "Unstarted review runtime missing or externally started"
        );
        p.start_attempted = true;
        p.deadline = now() + 300_000;
        save(j, r, &p)?;
        docker(&c, &["start", &c.container_id]).await?;
        observed = inspect(&c, &c.container_id).await?;
        let v = observed
            .as_ref()
            .context("Started review runtime missing")?;
        validate(v, &p, r)?;
        p.started_at = Some(
            v["State"]["StartedAt"]
                .as_str()
                .context("Start identity missing")?
                .into(),
        );
        save(j, r, &p)?;
    }
    if p.exit.is_none() {
        let v = observed
            .as_ref()
            .context("Test runtime disappeared before exit evidence")?;
        if p.started_at.is_none() {
            p.exit = Some(-1);
        } else if v["State"]["Running"] == true && now() < p.deadline {
            return Ok(None);
        } else {
            p.exit = Some(if v["State"]["Running"] == true {
                -1
            } else {
                v["State"]["ExitCode"]
                    .as_i64()
                    .context("Exit status missing")? as i32
            });
        }
        save(j, r, &p)?;
    }
    let log = p
        .work
        .parent()
        .unwrap()
        .join(format!("{}.test.log", r.command.id));
    if !log.exists() {
        let bytes = docker(&c, &["logs", &c.container_id])
            .await
            .unwrap_or_default();
        let mut opts = fs::OpenOptions::new();
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = opts.write(true).create_new(true).mode(0o600).open(&log)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    if observed.is_some() {
        ensure!(
            !p.remove_attempted,
            "Review removal uncertain; reconcile exact runtime before continuing"
        );
        p.remove_attempted = true;
        save(j, r, &p)?;
        let _ = docker(&c, &["rm", "--force", &c.container_id]).await;
    }
    ensure!(
        daemon(&c).await? == p.daemon && inspect(&c, &c.container_id).await?.is_none(),
        "Review runtime removal not confirmed"
    );
    if p.removed.is_none() {
        p.removed = Some(RemovalProof {
            runtime: RuntimeRef {
                container_id: c.container_id,
                started_at: p.started_at.clone().unwrap_or_else(|| "unconfirmed".into()),
            },
            daemon_id: p.daemon.clone(),
            observed_unix_ms: now(),
        });
        save(j, r, &p)?;
    }
    if p.exit != Some(0) {
        return Ok(error(format!(
            "Review tests failed or timed out (exit {:?}); private log: {}",
            p.exit,
            log.display()
        )));
    }
    if !fs::symlink_metadata(&p.work).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        || !fs::symlink_metadata(p.work.join(".git"))
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        || fs::read_to_string(p.work.join(".git")).ok().as_deref() != Some(&p.git_pointer)
    {
        return Ok(error(
            "Tests changed the review Git pointer; candidate was not approved",
        ));
    }
    if !git(&p.work, &["status", "--porcelain=v1"])?.is_empty() {
        return Ok(error(
            "Tests modified tracked files or left unignored artifacts; candidate was not approved",
        ));
    }
    let ReviewJob::Review {
        checkpoint,
        target_commit,
        ..
    } = job
    else {
        unreachable!()
    };
    Ok(Some(CommandOutput::Review {
        review: ReviewResult {
            checkpoint: checkpoint.command_id,
            target_commit: target_commit.clone(),
            candidate_commit: p.candidate,
            tree: p.tree,
            image: p.image,
            test_exit_code: 0,
            removal: p.removed.unwrap(),
            log_path: log.to_string_lossy().into(),
        },
    }))
}
