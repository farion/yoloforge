//! Explicit reviewed-result publication. External writes are journaled before dispatch.
//! An uncertain PR creation is reconciled by identity, never blindly repeated.
use crate::{
    workspace::{git, git_network},
    Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_protocol::client::FactoryClient;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Provider {
    Github,
    Forgejo,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    provider: Provider,
    api_url: String,
    repository: String,
    git_remote: String,
    token_file: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Destination {
    provider: Provider,
    api_url: String,
    repository: String,
    git_remote: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Identity {
    server: String,
    runner: RunnerId,
    review: CommandId,
    destination: Destination,
    repository: PathBuf,
    base: String,
    commit: String,
    head: String,
}
#[derive(Serialize, Deserialize)]
struct Receipt {
    request_id: RequestId,
    identity: Identity,
    push_attempted: bool,
    create_attempted: bool,
    url: Option<String>,
}
fn validate(config: &Config) -> Result<Destination> {
    let url = reqwest::Url::parse(&config.api_url)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Hosting API requires a credential-free HTTPS URL"
    );
    let parts = config.repository.split('/').collect::<Vec<_>>();
    ensure!(
        parts.len() == 2
            && parts.iter().all(|p| !p.is_empty()
                && !matches!(*p, "." | "..")
                && p.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))),
        "Repository must be owner/name"
    );
    let expected_host = if url.host_str() == Some("api.github.com") {
        "github.com"
    } else {
        url.host_str().unwrap()
    };
    // Both Git and hosting API must address the same host and repository.
    let (host, path) =
        if config.git_remote.starts_with("https://") || config.git_remote.starts_with("ssh://") {
            let remote = reqwest::Url::parse(&config.git_remote)?;
            ensure!(
                remote.password().is_none()
                    && remote.query().is_none()
                    && remote.fragment().is_none()
                    && (remote.scheme() == "ssh" || remote.username().is_empty()),
                "Git URL must not contain credentials"
            );
            (
                remote.host_str().context("Git host missing")?.to_string(),
                remote.path().trim_start_matches('/').to_string(),
            )
        } else {
            let (host, path) = config
                .git_remote
                .split_once(':')
                .context("Use an explicit SSH or HTTPS Git remote URL")?;
            ensure!(
                host.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-@".contains(&c)),
                "Invalid SSH Git remote host"
            );
            (
                host.rsplit('@').next().unwrap_or(host).to_string(),
                path.into(),
            )
        };
    ensure!(
        !config.git_remote.starts_with('-')
            && !config.git_remote.chars().any(|c| c.is_control())
            && host == expected_host
            && path.strip_suffix(".git").unwrap_or(&path) == config.repository,
        "Git remote differs from hosting API repository"
    );
    Ok(Destination {
        provider: config.provider.clone(),
        api_url: config.api_url.trim_end_matches('/').into(),
        repository: config.repository.clone(),
        git_remote: config.git_remote.clone(),
    })
}
fn store(path: &Path, receipt: &Receipt) -> Result<()> {
    let parent = path.parent().context("Receipt directory missing")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec_pretty(receipt)?)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
fn branch_tip(identity: &Identity) -> Result<Option<String>> {
    let reference = format!("refs/heads/{}", identity.head);
    let output = git_network(
        &identity.repository,
        &[
            "ls-remote",
            "--refs",
            "--",
            &identity.destination.git_remote,
            &reference,
        ],
    )?;
    let rows = output.lines().collect::<Vec<_>>();
    ensure!(rows.len() <= 1, "Ambiguous hosted branch");
    rows.first()
        .map(|row| {
            let (hash, name) = row
                .split_once('\t')
                .context("Invalid Git remote response")?;
            ensure!(name == reference, "Unexpected remote ref");
            Ok(hash.into())
        })
        .transpose()
}
fn push(path: &Path, receipt: &mut Receipt) -> Result<()> {
    if let Some(commit) = branch_tip(&receipt.identity)? {
        ensure!(
            commit == receipt.identity.commit,
            "Publication branch changed remotely; refusing to overwrite it"
        );
        return Ok(());
    }
    receipt.push_attempted = true;
    store(path, receipt)?;
    let i = &receipt.identity;
    // An absent-ref lease prevents racing another writer. No target-branch update.
    git_network(
        &i.repository,
        &[
            "push",
            &format!("--force-with-lease=refs/heads/{}:", i.head),
            "--",
            &i.destination.git_remote,
            &format!("{}:refs/heads/{}", i.commit, i.head),
        ],
    )?;
    ensure!(
        branch_tip(i)?.as_deref() == Some(i.commit.as_str()),
        "Remote publication not confirmed"
    );
    Ok(())
}
fn marker(i: &Identity) -> String {
    format!("<!-- yoloforge-review:{} commit:{} -->", i.review, i.commit)
}
fn match_pull(i: &Identity, value: &Value) -> Result<Option<String>> {
    if value["head"]["ref"] != i.head || value["base"]["ref"] != i.base {
        return Ok(None);
    }
    ensure!(
        value["body"]
            .as_str()
            .is_some_and(|b| b.contains(&marker(i))),
        "Existing pull request has different publication provenance"
    );
    ensure!(
        value["head"]["sha"] == i.commit
            && value["head"]["repo"]["full_name"] == i.destination.repository,
        "Pull request head no longer matches reviewed candidate"
    );
    let raw = value["html_url"]
        .as_str()
        .context("Pull request URL missing")?;
    let url = reqwest::Url::parse(raw)?;
    let api = reqwest::Url::parse(&i.destination.api_url)?;
    let host = if api.host_str() == Some("api.github.com") {
        Some("github.com")
    } else {
        api.host_str()
    };
    ensure!(
        url.scheme() == api.scheme()
            && url.host_str() == host
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url
                .path()
                .starts_with(&format!("/{}/pull", i.destination.repository)),
        "Unexpected pull request URL"
    );
    Ok(Some(raw.into()))
}
async fn decode(mut response: reqwest::Response) -> Result<Value> {
    ensure!(
        response.status().is_success(),
        "Hosting API returned HTTP {}",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 2 * 1024 * 1024,
            "Hosting response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}
async fn lookup(client: &reqwest::Client, token: &str, i: &Identity) -> Result<Option<String>> {
    let endpoint = format!(
        "{}/repos/{}/pulls",
        i.destination.api_url, i.destination.repository
    );
    for page in 1..=100 {
        let mut request = client
            .get(&endpoint)
            .bearer_auth(token)
            .query(&[("state", "all"), ("page", &page.to_string())]);
        request = match i.destination.provider {
            Provider::Github => request.query(&[
                ("per_page", "100"),
                (
                    "head",
                    &format!(
                        "{}:{}",
                        i.destination.repository.split('/').next().unwrap(),
                        i.head
                    ),
                ),
                ("base", &i.base),
            ]),
            Provider::Forgejo => request.query(&[("limit", "100")]),
        };
        let value = decode(request.send().await?).await?;
        let pulls = value.as_array().context("Invalid pull request list")?;
        let mut found = None;
        for pull in pulls {
            if let Some(url) = match_pull(i, pull)? {
                ensure!(found.is_none(), "Multiple matching pull requests");
                found = Some(url);
            }
        }
        if found.is_some() {
            return Ok(found);
        }
        if pulls.len() < 100 {
            return Ok(None);
        }
    }
    anyhow::bail!("Pull request search exceeded page limit; publication remains unconfirmed")
}
async fn publish_pull(
    client: &reqwest::Client,
    token: &str,
    path: &Path,
    receipt: &mut Receipt,
) -> Result<String> {
    if let Some(url) = &receipt.url {
        return Ok(url.clone());
    }
    if let Some(url) = lookup(client, token, &receipt.identity).await? {
        receipt.url = Some(url.clone());
        store(path, receipt)?;
        return Ok(url);
    }
    ensure!(!receipt.create_attempted,"Previous PR creation is uncertain. No matching PR found; refusing a duplicate POST. Inspect the hosting provider before manual recovery.");
    receipt.create_attempted = true;
    store(path, receipt)?;
    let i = &receipt.identity;
    let response=client.post(format!("{}/repos/{}/pulls",i.destination.api_url,i.destination.repository)).bearer_auth(token).json(&json!({"title":format!("YoloForge review {}",i.review),"head":i.head,"base":i.base,"body":format!("Reviewed candidate `{}`.\n\n{}",i.commit,marker(i))})).send().await?;
    let value = decode(response).await?;
    let url = match_pull(i, &value)?.context("Created pull request identity differs")?;
    receipt.url = Some(url.clone());
    store(path, receipt)?;
    Ok(url)
}
pub async fn publish(args: &Args, client: &FactoryClient, id: CommandId) -> Result<()> {
    let config: Config = serde_json::from_slice(&std::fs::read(
        args.hosting_config
            .as_ref()
            .context("Hosting configuration missing")?,
    )?)?;
    let destination = validate(&config)?;
    let journal = args
        .journal
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!(".factory-data/runner-{}.sqlite3", args.id)));
    let record = crate::attach::read_record(&journal, &args.server, args.id, id)?;
    let snapshot = client.snapshot().await?;
    let current = snapshot
        .commands
        .iter()
        .find(|c| c.id == id)
        .context("Review missing from Factory")?;
    ensure!(
        current.kind == CommandKind::ReviewCheckpoint
            && current.state == CommandState::Succeeded
            && current.result == record.result,
        "Only a centrally confirmed successful review can be published"
    );
    let owner = snapshot
        .streams
        .iter()
        .find(|s| s.id == current.stream_id)
        .context("Review stream missing")?;
    ensure!(
        owner.runner_id == args.id
            && current.runner_id == args.id
            && owner.ownership_epoch == current.ownership_epoch
            && current.ownership_epoch == record.command.ownership_epoch,
        "Review ownership changed"
    );
    let Some(CommandOutput::Review { review }) = &record.result else {
        anyhow::bail!("Review result missing");
    };
    ensure!(review.test_exit_code == 0, "Review tests failed");
    let (repository, base) = record
        .review
        .as_ref()
        .context("Local review evidence missing")?
        .hosting_source()?;
    let identity = Identity {
        server: args.server.trim_end_matches('/').into(),
        runner: args.id,
        review: id,
        destination,
        repository,
        base,
        commit: review.candidate_commit.clone(),
        head: format!("yoloforge/review-{id}"),
    };
    ensure!(
        git(
            &identity.repository,
            &[
                "rev-parse",
                "--verify",
                &format!("{}^{{commit}}", identity.commit)
            ]
        )?
        .trim()
            == identity.commit,
        "Reviewed candidate missing locally"
    );
    let directory = journal.parent().unwrap_or(Path::new(".")).join("hosting");
    std::fs::create_dir_all(&directory)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(format!("{id}.lock")))?;
    lock.try_lock_exclusive()
        .context("This review publication is already running")?;
    let path = directory.join(format!("{id}.json"));
    let mut receipt = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Receipt>(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Receipt {
            request_id: RequestId::new(),
            identity: identity.clone(),
            push_attempted: false,
            create_attempted: false,
            url: None,
        },
        Err(e) => return Err(e.into()),
    };
    ensure!(
        receipt.identity == identity,
        "Publication destination or review changed; refusing to retarget journaled writes"
    );
    if let Some(url) = &receipt.url {
        report(client, &receipt, current.ownership_epoch).await?;
        println!("{url}");
        return Ok(());
    }
    let token =
        std::fs::read_to_string(config.token_file).context("Hosting token file not readable")?;
    let token = token.trim();
    ensure!(
        !token.is_empty() && !token.chars().any(|c| c.is_control()),
        "Invalid hosting token file"
    );
    let http = reqwest::Client::builder()
        .user_agent("YoloForge")
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    tokio::task::block_in_place(|| push(&path, &mut receipt))?;
    let url = publish_pull(&http, token, &path, &mut receipt).await?;
    report(client, &receipt, current.ownership_epoch).await?;
    println!("{url}");
    Ok(())
}
async fn report(client: &FactoryClient, receipt: &Receipt, ownership_epoch: u64) -> Result<()> {
    let i = &receipt.identity;
    client
        .report_hosted_review(
            i.review,
            &factory_protocol::ReportHostedReview {
                request_id: receipt.request_id,
                ownership_epoch,
                result: HostedReview {
                    review_command: i.review,
                    candidate_commit: i.commit.clone(),
                    target_branch: i.base.clone(),
                    publication_branch: i.head.clone(),
                    repository: i.destination.repository.clone(),
                    url: receipt.url.clone().context("Hosting URL missing")?,
                },
            },
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
    use std::sync::{Arc, Mutex};
    fn identity(repository: PathBuf) -> Identity {
        Identity {
            server: "http://factory".into(),
            runner: RunnerId::new(),
            review: CommandId::new(),
            destination: Destination {
                provider: Provider::Github,
                api_url: "https://api.github.com".into(),
                repository: "owner/repo".into(),
                git_remote: "git@github.com:owner/repo.git".into(),
            },
            repository,
            base: "main".into(),
            commit: "a".repeat(40),
            head: "yoloforge/review-test".into(),
        }
    }
    fn receipt(identity: Identity) -> Receipt {
        Receipt {
            request_id: RequestId::new(),
            identity,
            push_attempted: false,
            create_attempted: false,
            url: None,
        }
    }
    #[test]
    fn destination_validation_rejects_credentials_and_different_git_hosts_or_repositories() {
        let mut config = Config {
            provider: Provider::Github,
            api_url: "https://api.github.com".into(),
            repository: "owner/repo".into(),
            git_remote: "git@github.com:owner/repo.git".into(),
            token_file: "/private/token".into(),
        };
        validate(&config).unwrap();
        for remote in [
            "git@other.test:owner/repo.git",
            "git@github.com:other/repo.git",
            "https://user:secret@github.com/owner/repo.git",
            "-oProxyCommand=bad",
        ] {
            config.git_remote = remote.into();
            assert!(validate(&config).is_err());
        }
        config.git_remote = "https://github.com/owner/repo.git".into();
        config.api_url = "https://secret@api.github.com".into();
        assert!(validate(&config).is_err());
    }
    #[test]
    fn pushed_candidate_reconciles_after_restart_and_never_overwrites_a_remote_writer() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let remote = dir.path().join("remote");
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir(&remote).unwrap();
        git(&repo, &["init", "--initial-branch=main"]).unwrap();
        git(&remote, &["init", "--bare"]).unwrap();
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "first",
            ],
        )
        .unwrap();
        let commit = git(&repo, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        let mut i = identity(repo.clone());
        i.commit = commit.clone();
        i.destination.git_remote = remote.to_str().unwrap().into();
        let mut receipt = receipt(i);
        let path = dir.path().join("receipt.json");
        push(&path, &mut receipt).unwrap();
        let mut receipt: Receipt = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(receipt.push_attempted);
        push(&path, &mut receipt).unwrap();
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]).unwrap().trim(), commit);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "external",
            ],
        )
        .unwrap();
        let changed = git(&repo, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        git(
            &repo,
            &[
                "push",
                remote.to_str().unwrap(),
                &format!("{changed}:refs/heads/{}", receipt.identity.head),
            ],
        )
        .unwrap();
        assert!(push(&path, &mut receipt).is_err());
        assert_eq!(branch_tip(&receipt.identity).unwrap(), Some(changed));
    }
    #[derive(Clone)]
    struct ApiState {
        i: Identity,
        counts: Arc<Mutex<(usize, bool)>>,
    }
    fn pull(i: &Identity) -> Value {
        json!({"html_url":format!("{}/owner/repo/pull/1",i.destination.api_url),"body":marker(i),"head":{"ref":i.head,"sha":i.commit,"repo":{"full_name":"owner/repo"}},"base":{"ref":i.base}})
    }
    async fn list(State(s): State<ApiState>) -> Json<Value> {
        let exists = s.counts.lock().unwrap().1;
        Json(if exists {
            json!([pull(&s.i)])
        } else {
            json!([])
        })
    }
    async fn create(State(s): State<ApiState>, Json(body): Json<Value>) -> StatusCode {
        assert_eq!(body["head"], s.i.head);
        assert_eq!(body["base"], s.i.base);
        assert!(body["body"].as_str().unwrap().contains(&marker(&s.i)));
        let mut state = s.counts.lock().unwrap();
        state.0 += 1;
        state.1 = true;
        StatusCode::INTERNAL_SERVER_ERROR
    }
    #[tokio::test]
    async fn lost_pr_response_is_reconciled_without_a_duplicate_post_for_both_providers() {
        for provider in [Provider::Github, Provider::Forgejo] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("receipt.json");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut i = identity(dir.path().into());
            i.destination.api_url = format!("http://{}", listener.local_addr().unwrap());
            i.destination.provider = provider;
            let counts = Arc::new(Mutex::new((0, false)));
            let state = ApiState {
                i: i.clone(),
                counts: counts.clone(),
            };
            let app = Router::new()
                .route("/repos/owner/repo/pulls", get(list).post(create))
                .with_state(state);
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let mut receipt = receipt(i);
            let http = reqwest::Client::new();
            assert!(publish_pull(&http, "test-token", &path, &mut receipt)
                .await
                .is_err());
            assert!(receipt.create_attempted);
            let mut receipt: Receipt =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let url = publish_pull(&http, "test-token", &path, &mut receipt)
                .await
                .unwrap();
            assert!(url.ends_with("/owner/repo/pull/1"));
            assert_eq!(counts.lock().unwrap().0, 1);
            assert_eq!(
                publish_pull(&http, "test-token", &path, &mut receipt)
                    .await
                    .unwrap(),
                url
            );
            assert_eq!(counts.lock().unwrap().0, 1);
            receipt.url = None;
            counts.lock().unwrap().1 = false;
            assert!(publish_pull(&http, "test-token", &path, &mut receipt)
                .await
                .is_err());
            assert_eq!(counts.lock().unwrap().0, 1);
            server.abort();
        }
    }
    #[test]
    fn pull_identity_rejects_changed_commits_and_foreign_provenance() {
        let i = identity("/repo".into());
        let mut value = pull(&i);
        value["html_url"] = json!("https://github.com/owner/repo/pull/1");
        assert!(match_pull(&i, &value).unwrap().is_some());
        value["head"]["sha"] = json!("b".repeat(40));
        assert!(match_pull(&i, &value).is_err());
        value["head"]["sha"] = json!(i.commit);
        value["body"] = json!("someone else's PR");
        assert!(match_pull(&i, &value).is_err());
    }
}
