//! Persist intent before external writes. After uncertain delivery, only read and reconcile.
use crate::{
    journal::{Journal, Record},
    workspace, Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_opencode::OpenCodeClient;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Execution {
    #[serde(default)]
    model: Option<(String, String)>,
    #[serde(default)]
    pub(crate) runtime: Option<crate::containment::Identity>,
    pub(crate) workspace: Workspace,
    pub(crate) endpoint: String,
    session_attempted: bool,
    pub(crate) prompt_attempted: bool,
    #[serde(default)]
    interrupt_attempted: bool,
    #[serde(default)]
    previous_assistant: Option<String>,
}

fn progress(
    command: &RunnerCommand,
    phase: AgentPhase,
    summary: impl Into<String>,
) -> AgentProgress {
    AgentProgress {
        session_id: command.session_id(),
        message_id: agent_message_id(command.id),
        phase,
        summary: summary.into(),
    }
}

pub async fn step(journal: &Journal, record: &mut Record, args: &Args) -> Result<AgentProgress> {
    record.question_snapshot = None;
    let command = record.command.clone();
    let Some(path) = args.opencode_registration.as_ref() else {
        return Ok(progress(
            &command,
            if record.agent.is_some() {
                AgentPhase::Unknown
            } else {
                AgentPhase::Failed
            },
            "OpenCode registration not configured",
        ));
    };
    let client = match OpenCodeClient::from_registration(path) {
        Ok(client) => client,
        Err(_) => {
            return Ok(progress(
                &command,
                AgentPhase::Unknown,
                "OpenCode registration unavailable",
            ))
        }
    };
    if !matches!(client.status().await, OpenCodeStatus::Ready { .. }) {
        return Ok(progress(
            &command,
            AgentPhase::Unknown,
            "Supported OpenCode service unavailable; no external write attempted",
        ));
    }
    if record.agent.is_none() {
        let existing = tokio::task::block_in_place(|| {
            workspace::existing(
                journal,
                &command,
                args.repositories.as_deref(),
                args.workspace_root
                    .as_deref()
                    .expect("workspace root initialized"),
            )
        });
        let workspace = match existing {
            Ok(workspace) => workspace,
            Err(_) => {
                return Ok(progress(
                    &command,
                    AgentPhase::Failed,
                    "Prepared workspace identity could not be verified",
                ))
            }
        };
        let runtime = match crate::containment::bind(
            args,
            &command,
            client.endpoint(),
            &workspace.path,
        )
        .await
        {
            Ok(runtime) => runtime,
            Err(_) => {
                return Ok(progress(
                    &command,
                    AgentPhase::Unknown,
                    "Dedicated runtime not verified; no session or prompt created",
                ))
            }
        };
        record.agent = Some(Execution {
            model: None,
            runtime,
            workspace,
            endpoint: client.endpoint().into(),
            session_attempted: false,
            prompt_attempted: false,
            interrupt_attempted: false,
            previous_assistant: None,
        });
        tokio::task::block_in_place(|| journal.save(record))?;
    }
    // Network/protocol uncertainty never releases this stream for another writer.
    if record.agent.as_ref().is_some_and(|a| a.runtime.is_some()) && !record.runtime_published {
        return Ok(progress(
            &command,
            AgentPhase::Starting,
            "Awaiting durable Factory runtime binding before agent writes",
        ));
    }
    if args.managed_opencode.is_some() {
        let execution = record.agent.as_ref().expect("execution initialized");
        if !execution.prompt_attempted {
            if let Err(e) = tokio::task::block_in_place(|| {
                crate::managed_opencode::validate_workspace(std::path::Path::new(&execution.workspace.path))
            }) {
                return Ok(progress(&command, AgentPhase::Failed, e.to_string().chars().take(240).collect::<String>()));
            }
        }
    }
    match advance(&client, journal, record, args).await {
        Ok(value) => Ok(value),
        Err(_) => Ok(progress(
            &command,
            AgentPhase::Unknown,
            "OpenCode outcome uncertain; reconciling without resubmission",
        )),
    }
}

/// Read-only sampling. A failed read must leave the previous durable sample untouched.
pub async fn capture(record: &mut Record, args: &Args) -> Result<()> {
    crate::containment::ready(record, args).await?;
    let execution = record.agent.as_ref().context("No agent session yet")?;
    if !execution.prompt_attempted {
        return Ok(());
    }
    let client = OpenCodeClient::from_registration(
        args.opencode_registration
            .as_ref()
            .context("No OpenCode registration")?,
    )?;
    ensure!(
        execution.endpoint == client.endpoint(),
        "OpenCode endpoint changed"
    );
    let id = record.command.session_id();
    let session = client.session(&id).await?.context("Session missing")?;
    ensure!(
        session["id"] == id
            && session["metadata"]["factory_command"] == record.command.session_root().to_string()
            && session["location"]["directory"] == execution.workspace.path,
        "Session identity changed"
    );
    let (messages, older_omitted) = client.excerpt(&id).await?;
    if messages.is_empty() {
        return Ok(());
    }
    let revision = record
        .excerpt
        .as_ref()
        .map_or(0, |v| v.revision.0)
        .checked_add(1)
        .context("Excerpt revision exhausted")?;
    let captured_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;
    record.excerpt = Some(SessionExcerpt {
        session_title: session["title"]
            .as_str()
            .map(|v| {
                v.trim()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(512)
                    .collect()
            })
            .filter(|v: &String| !v.is_empty()),
        session_id: id,
        revision: Revision(revision),
        captured_unix_ms,
        older_omitted,
        messages,
    });
    Ok(())
}

pub async fn pause(
    journal: &Journal,
    record: &mut Record,
    args: &Args,
) -> Result<(PauseState, String)> {
    ensure!(
        record
            .agent
            .as_ref()
            .is_none_or(|a| a.runtime.is_none() || !a.prompt_attempted),
        "Contained execution requires freezer control, never native interrupt"
    );
    if record.agent.as_ref().is_none_or(|a| !a.prompt_attempted) {
        return Ok((
            PauseState::HeldBeforeExecution,
            "Held before prompt submission; no Factory execution started".into(),
        ));
    }
    let summary = "Interrupt intent recorded; execution/process quiescence unverified";
    let Some(path) = args.opencode_registration.as_ref() else {
        return Ok((
            PauseState::InterruptUnverified,
            "OpenCode registration unavailable; stop unverified".into(),
        ));
    };
    let client = OpenCodeClient::from_registration(path)?;
    ensure!(
        matches!(client.status().await, OpenCodeStatus::Ready { .. }),
        "Supported OpenCode service unavailable"
    );
    let execution = record.agent.as_ref().unwrap();
    ensure!(
        execution.endpoint == client.endpoint(),
        "OpenCode endpoint changed"
    );
    let session = client.session(&record.command.session_id()).await?;
    ensure!(
        session
            .as_ref()
            .is_some_and(|s| s["metadata"]["factory_command"]
                == record.command.session_root().to_string()
                && s["location"]["directory"] == execution.workspace.path),
        "Session identity changed"
    );
    if !record.agent.as_ref().unwrap().interrupt_attempted {
        record.agent.as_mut().unwrap().interrupt_attempted = true;
        tokio::task::block_in_place(|| journal.save(record))?;
        let _ = client.interrupt(&record.command.session_id()).await;
    }
    // Reconcile even after a lost interrupt response or runner restart. Never
    // infer process-tree quiescence from the native session's terminal outcome.
    let id = record.command.session_id();
    let observed = client.session(&id).await?;
    let execution = record.agent.as_ref().unwrap();
    let spec = record
        .command
        .agent_spec
        .as_ref()
        .context("Agent specification missing")?;
    if observed.is_some_and(|s| {
        s["outcome"] == "interrupted"
            && s["id"] == id
            && s["metadata"]["factory_command"] == record.command.session_root().to_string()
            && s["location"]["directory"] == execution.workspace.path
            && s["model"]["providerID"] == spec.profile.provider_id
            && s["model"]["id"] == spec.profile.model_id
    }) && client
        .message(&id, &agent_message_id(record.command.id))
        .await?
        .is_some_and(|m| m["type"] == "user" && m["text"] == spec.prompt)
        && !client.active(&id).await?
        && client
            .session_items(&id, "inbox")
            .await?
            .as_array()
            .is_some_and(|items| items.is_empty())
    {
        return Ok((
            PauseState::SessionInterrupted,
            "Native session interrupted; background processes not verified stopped".into(),
        ));
    }
    Ok((PauseState::InterruptUnverified, summary.into()))
}

async fn advance(
    client: &OpenCodeClient,
    journal: &Journal,
    record: &mut Record,
    args: &Args,
) -> Result<AgentProgress> {
    let command = record.command.clone();
    crate::containment::ready(record, args).await?;
    let spec = command
        .agent_spec
        .as_ref()
        .context("Agent specification missing")?;
    if spec.profile.provider_id.trim().is_empty() || spec.profile.model_id.trim().is_empty() {
        return Ok(progress(&command, AgentPhase::Unknown, "Explicit project provider and model required; default model fallback is forbidden. No session or prompt sent."));
    }
    let configured = (
        spec.profile.provider_id.clone(),
        spec.profile.model_id.clone(),
    );
    if record
        .agent
        .as_ref()
        .and_then(|a| a.model.as_ref())
        .is_some_and(|model| model != &configured)
    {
        return Ok(progress(
            &command,
            AgentPhase::Unknown,
            "Journal model differs from explicit project model; no session or prompt sent.",
        ));
    }
    if record
        .agent
        .as_ref()
        .context("Agent journal missing")?
        .model
        .is_none()
    {
        record.agent.as_mut().unwrap().model = Some(configured);
        tokio::task::block_in_place(|| journal.save(record))?;
    }
    let execution = record
        .agent
        .as_ref()
        .context("Agent journal missing")?
        .clone();
    ensure!(
        execution.endpoint == client.endpoint(),
        "OpenCode endpoint changed"
    );
    let session_id = command.session_id();
    let (provider, model) = execution.model.as_ref().context("Selected model missing")?;
    let message_id = agent_message_id(command.id);
    let session = client.session(&session_id).await?;
    let Some(session) = session else {
        ensure!(
            spec.continuation.is_none(),
            "Continuation session missing; never recreate it"
        );
        if execution.session_attempted {
            record.question_snapshot = Some(Vec::new());
        }
        ensure!(
            !execution.session_attempted,
            "Session creation outcome unknown or session removed"
        );
        record.agent.as_mut().unwrap().session_attempted = true;
        tokio::task::block_in_place(|| journal.save(record))?;
        client
            .create_session(
                &session_id,
                &execution.workspace.path,
                &command.id.to_string(),
                provider,
                model,
                spec.read_only,
            )
            .await?;
        return Ok(progress(
            &command,
            AgentPhase::Starting,
            "Session created; prompt not submitted yet",
        ));
    };
    ensure!(
        session["id"] == session_id
            && session["metadata"]["factory_command"] == command.session_root().to_string()
            && session["location"]["directory"] == execution.workspace.path
            && session["model"]["providerID"] == *provider
            && session["model"]["id"] == *model,
        "Session identity changed"
    );
    if !execution.prompt_attempted {
        if let Some(continuation) = &spec.continuation {
            let previous = journal
                .record(continuation.previous_command_id)?
                .context("Continuation predecessor journal missing")?;
            let prior = previous
                .agent
                .as_ref()
                .context("Predecessor execution missing")?;
            ensure!(
                previous.command.stream_id == command.stream_id
                    && previous.command.runner_id == command.runner_id
                    && previous.command.ownership_epoch == command.ownership_epoch
                    && previous.command.session_root() == command.session_root()
                    && previous.archive.finished
                    && matches!(previous.result,Some(CommandOutput::Agent {ref agent}) if matches!(agent.phase, AgentPhase::Succeeded | AgentPhase::Interrupted))
                    && prior.endpoint == execution.endpoint
                    && prior.workspace == execution.workspace
                    && prior.runtime == execution.runtime,
                "Continuation predecessor identity or completion differs"
            );
            ensure!(
                !client.active(&session_id).await?,
                "Session already active before continuation"
            );
            for kind in ["inbox", "form", "permission"] {
                ensure!(
                    client
                        .session_items(&session_id, kind)
                        .await?
                        .as_array()
                        .context("Invalid session items")?
                        .is_empty(),
                    "Session has pending work before continuation"
                );
            }
            let messages = client
                .session_items(&session_id, "message?type=assistant&order=desc&limit=1")
                .await?;
            let previous_assistant = messages
                .as_array()
                .context("Invalid assistant messages")?
                .first()
                .and_then(|m| m["id"].as_str())
                .map(String::from);
            ensure!(
                previous_assistant.is_some()
                    || matches!(&previous.result, Some(CommandOutput::Agent { agent }) if agent.phase == AgentPhase::Interrupted),
                "Previous assistant missing"
            );
            record.agent.as_mut().unwrap().previous_assistant = previous_assistant;
        }
        if spec.read_only {
            ensure!(
                session["permissions"]
                    == serde_json::json!([{"action":"*","resource":"*","effect":"deny"}]),
                "Supervisor tool permissions changed"
            );
        }
        let current = tokio::task::block_in_place(|| {
            workspace::existing(
                journal,
                &command,
                args.repositories.as_deref(),
                args.workspace_root
                    .as_deref()
                    .expect("workspace root initialized"),
            )
        })?;
        ensure!(
            current == execution.workspace,
            "Workspace identity changed before prompt"
        );
        record.agent.as_mut().unwrap().prompt_attempted = true;
        tokio::task::block_in_place(|| journal.save(record))?;
        client
            .prompt_with_files(
                &session_id,
                &message_id,
                &spec.prompt,
                &spec.images,
                &spec.files,
            )
            .await?;
        return Ok(progress(
            &command,
            AgentPhase::Running,
            "Prompt durably admitted",
        ));
    }
    let message = client.message(&session_id, &message_id).await?;
    let prompt_created = message
        .as_ref()
        .and_then(|m| m.pointer("/time/created"))
        .and_then(|v| v.as_f64());
    let admitted = if let Some(message) = &message {
        ensure!(
            message["text"] == spec.prompt && message["type"] == "user",
            "Projected prompt differs"
        );
        ensure!(
            spec.images.is_empty()
                || message["metadata"]["factory_images"] == image_digest(&spec.images),
            "Projected image prompt differs"
        );
        ensure!(
            spec.files.is_empty()
                || message["metadata"]["factory_files"] == files_digest(&spec.files),
            "Projected file prompt differs"
        );
        true
    } else {
        let inbox = client.session_items(&session_id, "inbox").await?;
        let pending = inbox
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == message_id);
        if let Some(pending) = pending {
            ensure!(
                pending["payload"]["text"] == spec.prompt,
                "Inbox prompt differs"
            );
            ensure!(
                spec.images.is_empty()
                    || pending["payload"]["metadata"]["factory_images"]
                        == image_digest(&spec.images),
                "Inbox image prompt differs"
            );
            ensure!(
                spec.files.is_empty()
                    || pending["payload"]["metadata"]["factory_files"] == files_digest(&spec.files),
                "Inbox file prompt differs"
            );
        }
        false
    };
    let forms = client.session_items(&session_id, "form").await?;
    let permissions = client.session_items(&session_id, "permission").await?;
    let mut questions = Vec::new();
    for (kind, items) in [
        (QuestionKind::Form, &forms),
        (QuestionKind::Permission, &permissions),
    ] {
        for payload in items.as_array().unwrap() {
            questions.push(ExternalQuestion {
                kind,
                external_id: payload["id"]
                    .as_str()
                    .context("Question ID missing")?
                    .into(),
                payload: payload.clone(),
            });
        }
    }
    record.question_snapshot = Some(questions);
    let waiting = forms.as_array().unwrap().len() + permissions.as_array().unwrap().len();
    if waiting > 0 {
        return Ok(progress(
            &command,
            AgentPhase::WaitingInput,
            format!("{waiting} pending OpenCode questions/permissions; session {session_id}"),
        ));
    }
    if client.active(&session_id).await? {
        return Ok(progress(
            &command,
            AgentPhase::Running,
            "OpenCode execution active",
        ));
    }
    ensure!(
        admitted,
        "Prompt not projected; admission or execution uncertain"
    );
    let messages = client
        .session_items(&session_id, "message?type=assistant&order=desc&limit=1")
        .await?;
    let last = messages.as_array().unwrap().first();
    if spec.continuation.is_some() {
        ensure!(
            last.is_some_and(
                |m| m["id"].as_str() != execution.previous_assistant.as_deref()
                    && m.pointer("/time/created")
                        .and_then(|v| v.as_f64())
                        .zip(prompt_created)
                        .is_some_and(|(reply, prompt)| reply >= prompt)
            ),
            "No response belonging to continuation observed"
        );
    }
    if session["outcome"] == "interrupted" {
        return Ok(progress(
            &command,
            AgentPhase::Unknown,
            "Execution interrupted; process quiescence not verified, no automatic resume",
        ));
    }
    if session["outcome"] == "failed" {
        return Ok(progress(
            &command,
            AgentPhase::Failed,
            "OpenCode execution failed; inspect session history",
        ));
    }
    if let Some(last) = last {
        if session["outcome"] == "succeeded"
            && last["finish"] == "stop"
            && last
                .pointer("/time/completed")
                .is_some_and(|v| v.is_number())
        {
            let summary = last["content"]
                .as_array()
                .context("Assistant content missing")?
                .iter()
                .filter(|p| p["type"] == "text")
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            // Byte-bound the wire result without splitting UTF-8 characters.
            let mut end = summary.len().min(24000);
            while !summary.is_char_boundary(end) {
                end -= 1;
            }
            return Ok(progress(
                &command,
                AgentPhase::Succeeded,
                summary[..end].to_string(),
            ));
        }
    }
    Ok(progress(
        &command,
        AgentPhase::Unknown,
        "Session inactive without confirmed final result; no automatic resume",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{Request, State},
        response::IntoResponse,
        Json, Router,
    };
    use serde_json::json;
    use std::{
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    #[derive(Clone)]
    struct Fake {
        command: RunnerCommand,
        mode: Arc<AtomicUsize>,
        posts: Arc<AtomicUsize>,
    }
    async fn api(State(fake): State<Fake>, request: Request) -> axum::response::Response {
        if request.method() != axum::http::Method::GET {
            fake.posts.fetch_add(1, Ordering::SeqCst);
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        let path = request.uri().path();
        let id = agent_session_id(fake.command.id);
        let spec = fake.command.agent_spec.as_ref().unwrap();
        let mode = fake.mode.load(Ordering::SeqCst);
        let value = if path == "/api/info" {
            json!({"version":"2.0.27"})
        } else if path == "/api/model/default" {
            json!({"location":{"directory":"/prepared"},"data":{"providerID":"openai","id":if mode==0 {"test"}else{"changed-default"}}})
        } else if path == "/api/session/active" {
            if mode == 6 {
                json!({"data":{id.clone():{}}})
            } else {
                json!({"data":{}})
            }
        } else if path == format!("/api/session/{id}") {
            if mode == 0 {
                return axum::http::StatusCode::NOT_FOUND.into_response();
            }
            json!({"data":{"id":id,"title":"OpenCode: Implement archive capture","metadata":{"factory_command":fake.command.id},"location":{"directory":"/prepared"},
                "model":{"providerID":spec.profile.provider_id,"id":spec.profile.model_id},"outcome":if matches!(mode,4|6|7|8) {"interrupted"} else {"succeeded"}}})
        } else if path.ends_with("/inbox") && mode == 7 {
            json!({"data":[{"id":"msg_pending"}]})
        } else if path.ends_with(&format!("/message/{}", agent_message_id(fake.command.id))) {
            if matches!(mode, 1 | 8) {
                return axum::http::StatusCode::NOT_FOUND.into_response();
            }
            json!({"data":{"type":"user","text":if mode==3 {"different"} else {&spec.prompt}}})
        } else if path.ends_with("/message") {
            if request.uri().query() == Some("order=desc&limit=32") {
                return Json(if mode==5 {json!({"invalid":true})} else {json!({"data":[{"id":"msg_excerpt","type":"assistant","content":[{"type":"text","text":"durable output"}]}],"cursor":{"next":null}})}).into_response();
            }
            json!({"data":[{"type":"assistant","finish":"stop","time":{"completed":1},"content":[{"type":"text","text":"Finished and tested"}]}]})
        } else {
            json!({"data":[]})
        };
        Json(value).into_response()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restart_reconciles_uncertain_session_and_prompt_without_reposting() {
        reconcile(false).await;
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn missing_explicit_model_never_uses_default_even_after_restart() {
        reconcile(true).await;
    }
    async fn reconcile(automatic_model: bool) {
        let dir = tempfile::tempdir().unwrap();
        let runner = RunnerId::new();
        let stream = StreamId::new();
        let mut command = RunnerCommand {
            id: CommandId::new(),
            stream_id: stream,
            runner_id: runner,
            ownership_epoch: 1,
            kind: CommandKind::RunAgent,
            state: CommandState::Accepted,
            result: None,
            workspace_spec: Some(WorkspaceSpec {
                setup: None,
                checkpoint: None,
                project_id: ProjectId::new(),
                branch: format!("factory/{stream}"),
                base_branch: "main".into(),
            }),
            agent_spec: Some(AgentSpec {
                files: vec![],
                role: String::new(),
                images: vec![],
                read_only: false,
                continuation: None,
                prompt: "Implement it".into(),
                profile: ExecutionProfile {
                    provider_id: "openai".into(),
                    model_id: "test".into(),
                    ..Default::default()
                },
            }),
            agent_progress: None,
            created_unix_ms: 1,
        };
        let mode = Arc::new(AtomicUsize::new(0));
        let posts = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().fallback(api).with_state(Fake {
            command: command.clone(),
            mode: mode.clone(),
            posts: posts.clone(),
        });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let registration = dir.path().join("registration.json");
        std::fs::write(
            &registration,
            serde_json::to_vec(&json!({"url":endpoint,"password":"test"})).unwrap(),
        )
        .unwrap();
        let path = dir.path().join("journal.db");
        if automatic_model {
            let profile = &mut command.agent_spec.as_mut().unwrap().profile;
            profile.provider_id.clear();
            profile.model_id.clear();
        }
        let journal = Journal::open(&path, "http://localhost:7432", runner).unwrap();
        let mut record = journal.receive(command.clone()).unwrap();
        record.agent = Some(Execution {
            runtime: None,
            model: None,
            previous_assistant: None,
            workspace: Workspace {
                path: "/prepared".into(),
                branch: format!("factory/{stream}"),
                base_commit: "a".repeat(40),
            },
            endpoint,
            session_attempted: true,
            prompt_attempted: true,
            interrupt_attempted: false,
        });
        journal.save(&record).unwrap();
        drop(journal);
        let journal = Journal::open(&path, "http://localhost:7432", runner).unwrap();
        let mut record = journal.receive(command).unwrap();
        let args = Args {
            open_session: None,
            fetch_repository: None,
            session_readiness: false,
            inspect_model: None,
            publish_review: None,
            hosting_config: None,
            archive_source_root: Vec::new(),
            rescan_archive_files: None,
            watch_publications: None,
            publish_result: None,
            review_image: None,
            review_docker_socket: "/var/run/docker.sock".into(),
            containment: None,
            server: "http://localhost:7432".into(),
            id: runner,
            name: "test".into(),
            location: crate::Location::Local,
            opencode_registration: Some(registration),
            managed_opencode: None,
            journal: Some(path),
            repositories: None,
            workspace_root: Some(PathBuf::from("/prepared")),
        };
        assert_eq!(
            step(&journal, &mut record, &args).await.unwrap().phase,
            AgentPhase::Unknown
        );
        mode.store(1, Ordering::SeqCst);
        assert_eq!(
            step(&journal, &mut record, &args).await.unwrap().phase,
            AgentPhase::Unknown
        );
        mode.store(3, Ordering::SeqCst);
        assert_eq!(
            step(&journal, &mut record, &args).await.unwrap().phase,
            AgentPhase::Unknown
        );
        mode.store(4, Ordering::SeqCst);
        assert_eq!(
            step(&journal, &mut record, &args).await.unwrap().phase,
            AgentPhase::Unknown
        );
        mode.store(2, Ordering::SeqCst);
        let result = step(&journal, &mut record, &args).await.unwrap();
        if automatic_model {
            assert_eq!(result.phase, AgentPhase::Unknown);
            assert!(result
                .summary
                .contains("Explicit project provider and model required"));
            assert!(record.agent.as_ref().unwrap().model.is_none());
            assert_eq!(posts.load(Ordering::SeqCst), 0);
            server.abort();
            return;
        }
        assert_eq!(result.phase, AgentPhase::Succeeded);
        assert_eq!(result.summary, "Finished and tested");
        assert_eq!(posts.load(Ordering::SeqCst), 0);
        capture(&mut record, &args).await.unwrap();
        let excerpt = record.excerpt.clone().unwrap();
        assert_eq!(
            excerpt.session_title.as_deref(),
            Some("OpenCode: Implement archive capture")
        );
        assert_eq!(excerpt.messages[0].text, "durable output\n");
        journal.save(&record).unwrap();
        mode.store(5, Ordering::SeqCst);
        assert!(capture(&mut record, &args).await.is_err());
        assert_eq!(record.excerpt, Some(excerpt.clone()));
        mode.store(2, Ordering::SeqCst);
        assert_eq!(
            pause(&journal, &mut record, &args).await.unwrap().0,
            PauseState::InterruptUnverified
        );
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        let command = record.command.clone();
        drop(journal);
        let journal = Journal::open(args.journal.as_ref().unwrap(), &args.server, runner).unwrap();
        let mut record = journal.receive(command).unwrap();
        assert_eq!(record.excerpt, Some(excerpt));
        assert_eq!(
            pause(&journal, &mut record, &args).await.unwrap().0,
            PauseState::InterruptUnverified
        );
        assert_eq!(
            posts.load(Ordering::SeqCst),
            1,
            "uncertain interrupt must not be reposted after restart"
        );
        for blocked in [6, 7, 8] {
            mode.store(blocked, Ordering::SeqCst);
            assert_eq!(
                pause(&journal, &mut record, &args).await.unwrap().0,
                PauseState::InterruptUnverified
            );
        }
        mode.store(4, Ordering::SeqCst);
        assert_eq!(
            pause(&journal, &mut record, &args).await.unwrap().0,
            PauseState::SessionInterrupted,
            "inactive interrupted session reconciles the lost response after restart"
        );
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        record.agent.as_mut().unwrap().prompt_attempted = false;
        assert_eq!(
            pause(&journal, &mut record, &args).await.unwrap().0,
            PauseState::HeldBeforeExecution
        );
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        server.abort();
    }
}
