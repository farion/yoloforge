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
    workspace: Workspace,
    pub(crate) endpoint: String,
    session_attempted: bool,
    prompt_attempted: bool,
    #[serde(default)]
    interrupt_attempted: bool,
}

fn progress(
    command: &RunnerCommand,
    phase: AgentPhase,
    summary: impl Into<String>,
) -> AgentProgress {
    AgentProgress {
        session_id: agent_session_id(command.id),
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
        record.agent = Some(Execution {
            workspace,
            endpoint: client.endpoint().into(),
            session_attempted: false,
            prompt_attempted: false,
            interrupt_attempted: false,
        });
        tokio::task::block_in_place(|| journal.save(record))?;
    }
    // Network/protocol uncertainty never releases this stream for another writer.
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
    let id = agent_session_id(record.command.id);
    let session = client.session(&id).await?.context("Session missing")?;
    ensure!(
        session["id"] == id
            && session["metadata"]["factory_command"] == record.command.id.to_string()
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
    if record.agent.as_ref().is_none_or(|a| !a.prompt_attempted) {
        return Ok((
            PauseState::HeldBeforeExecution,
            "Held before prompt submission; no Factory execution started".into(),
        ));
    }
    let summary = "Interrupt intent recorded; execution/process quiescence unverified";
    if record.agent.as_ref().unwrap().interrupt_attempted {
        return Ok((PauseState::InterruptUnverified, summary.into()));
    }
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
    let session = client.session(&agent_session_id(record.command.id)).await?;
    ensure!(
        session
            .as_ref()
            .is_some_and(
                |s| s["metadata"]["factory_command"] == record.command.id.to_string()
                    && s["location"]["directory"] == execution.workspace.path
            ),
        "Session identity changed"
    );
    record.agent.as_mut().unwrap().interrupt_attempted = true;
    tokio::task::block_in_place(|| journal.save(record))?;
    let _ = client.interrupt(&agent_session_id(record.command.id)).await;
    Ok((PauseState::InterruptUnverified, summary.into()))
}

async fn advance(
    client: &OpenCodeClient,
    journal: &Journal,
    record: &mut Record,
    args: &Args,
) -> Result<AgentProgress> {
    let command = record.command.clone();
    let spec = command
        .agent_spec
        .as_ref()
        .context("Agent specification missing")?;
    let execution = record
        .agent
        .as_ref()
        .context("Agent journal missing")?
        .clone();
    ensure!(
        execution.endpoint == client.endpoint(),
        "OpenCode endpoint changed"
    );
    let session_id = agent_session_id(command.id);
    let message_id = agent_message_id(command.id);
    let session = client.session(&session_id).await?;
    let Some(session) = session else {
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
                &spec.profile.provider_id,
                &spec.profile.model_id,
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
            && session["metadata"]["factory_command"] == command.id.to_string()
            && session["location"]["directory"] == execution.workspace.path
            && session["model"]["providerID"] == spec.profile.provider_id
            && session["model"]["id"] == spec.profile.model_id,
        "Session identity changed"
    );
    if !execution.prompt_attempted {
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
            .prompt(&session_id, &message_id, &spec.prompt)
            .await?;
        return Ok(progress(
            &command,
            AgentPhase::Running,
            "Prompt durably admitted",
        ));
    }
    let message = client.message(&session_id, &message_id).await?;
    let admitted = if let Some(message) = message {
        ensure!(
            message["text"] == spec.prompt && message["type"] == "user",
            "Projected prompt differs"
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
        } else if path == "/api/session/active" {
            json!({"data":{}})
        } else if path == format!("/api/session/{id}") {
            if mode == 0 {
                return axum::http::StatusCode::NOT_FOUND.into_response();
            }
            json!({"data":{"id":id,"metadata":{"factory_command":fake.command.id},"location":{"directory":"/prepared"},
                "model":{"providerID":spec.profile.provider_id,"id":spec.profile.model_id},"outcome":if mode==4 {"interrupted"} else {"succeeded"}}})
        } else if path.ends_with(&format!("/message/{}", agent_message_id(fake.command.id))) {
            if mode == 1 {
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
        let dir = tempfile::tempdir().unwrap();
        let runner = RunnerId::new();
        let stream = StreamId::new();
        let command = RunnerCommand {
            id: CommandId::new(),
            stream_id: stream,
            runner_id: runner,
            ownership_epoch: 1,
            kind: CommandKind::RunAgent,
            state: CommandState::Accepted,
            result: None,
            workspace_spec: Some(WorkspaceSpec {
                project_id: ProjectId::new(),
                branch: format!("factory/{stream}"),
                base_branch: "main".into(),
            }),
            agent_spec: Some(AgentSpec {
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
        let journal = Journal::open(&path, "http://localhost:7432", runner).unwrap();
        let mut record = journal.receive(command.clone()).unwrap();
        record.agent = Some(Execution {
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
            server: "http://localhost:7432".into(),
            id: runner,
            name: "test".into(),
            location: crate::Location::Local,
            opencode_registration: Some(registration),
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
        assert_eq!(result.phase, AgentPhase::Succeeded);
        assert_eq!(result.summary, "Finished and tested");
        assert_eq!(posts.load(Ordering::SeqCst), 0);
        capture(&mut record, &args).await.unwrap();
        let excerpt = record.excerpt.clone().unwrap();
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
        record.agent.as_mut().unwrap().prompt_attempted = false;
        assert_eq!(
            pause(&journal, &mut record, &args).await.unwrap().0,
            PauseState::HeldBeforeExecution
        );
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        server.abort();
    }
}
