//! Read-only evidence for commands whose write-ahead history has been lost.
//! Observations never reconstruct intent, authorize retries, or release ownership.
use crate::{journal::Record, Args};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_opencode::OpenCodeClient;

pub async fn observe(record: &Record, args: &Args) -> AgentProgress {
    let detail = match inspect(record, args).await {
        Ok(detail) => detail,
        Err(_) => "Native evidence unavailable or identity differs".into(),
    };
    AgentProgress {
        session_id: record.command.session_id(),
        message_id: agent_message_id(record.command.id),
        phase: AgentPhase::Unknown,
        summary: format!("Execution journal missing; quarantined without external writes. {detail}. Restore the owning runner journal; observation alone cannot prove prior writes or authorize replay."),
    }
}

async fn inspect(record: &Record, args: &Args) -> Result<String> {
    let command = &record.command;
    let spec = command
        .agent_spec
        .as_ref()
        .context("Agent specification missing")?;
    ensure!(
        !spec.profile.provider_id.is_empty() && !spec.profile.model_id.is_empty(),
        "Explicit model missing"
    );
    let client = OpenCodeClient::from_registration(
        args.opencode_registration
            .as_ref()
            .context("Registration missing")?,
    )?;
    let id = command.session_id();
    let Some(session) = client.session(&id).await? else {
        return Ok("Native session absent; prior execution remains unprovable".into());
    };
    let root = args
        .workspace_root
        .as_ref()
        .context("Workspace root missing")?;
    let key = if command.ownership_epoch == 1 {
        command.stream_id.to_string()
    } else {
        format!("{}-epoch-{}", command.stream_id, command.ownership_epoch)
    };
    let expected = root.join(key);
    let expected = expected.to_str().context("Workspace is not UTF-8")?;
    ensure!(
        session["id"] == id
            && session["metadata"]["factory_command"] == command.session_root().to_string()
            && session["location"]["directory"] == expected
            && session["model"]["providerID"] == spec.profile.provider_id
            && session["model"]["id"] == spec.profile.model_id,
        "Native identity differs"
    );
    let prompt = client.message(&id, &agent_message_id(command.id)).await?;
    let projected = prompt.as_ref().is_some_and(|m| {
        m["type"] == "user"
            && m["text"] == spec.prompt
            && (spec.images.is_empty()
                || m["metadata"]["factory_images"] == image_digest(&spec.images))
    });
    ensure!(prompt.is_none() || projected, "Projected prompt differs");
    let active = client.active(&id).await?;
    let inbox = client.session_items(&id, "inbox").await?;
    let pending = inbox.as_array().context("Invalid inbox")?.len();
    let outcome = match session["outcome"].as_str() {
        Some("succeeded") => "succeeded",
        Some("failed") => "failed",
        Some("interrupted") => "interrupted",
        _ => "unconfirmed",
    };
    Ok(format!("Native identity matches; prompt_projected={projected}, active={active}, inbox={pending}, outcome={outcome}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Request, response::IntoResponse, Json, Router};
    use clap::Parser;
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    };

    #[tokio::test(flavor = "multi_thread")]
    async fn lost_journal_observation_is_read_only_and_never_releases_success() {
        let dir = tempfile::tempdir().unwrap();
        let runner = RunnerId::new();
        let stream = StreamId::new();
        let id = CommandId::new();
        let session = agent_session_id(id);
        let root = dir.path().join("work");
        let workspace = root.join(stream.to_string());
        let writes = Arc::new(AtomicUsize::new(0));
        let mismatch = Arc::new(AtomicBool::new(false));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let w = writes.clone();
        let m = mismatch.clone();
        let app = Router::new().fallback(move |request: Request| {
            let session = session.clone(); let w = w.clone(); let m = m.clone();
            let workspace = workspace.clone();
            async move {
                if request.method() != axum::http::Method::GET {
                    w.fetch_add(1, Ordering::SeqCst);
                    return axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response();
                }
                let path = request.uri().path();
                Json(if path.ends_with(&format!("/session/{session}")) {
                    json!({"data":{"id":session,"metadata":{"factory_command":id},"location":{"directory":workspace},"model":{"providerID":"openai","id":if m.load(Ordering::SeqCst) {"other"} else {"explicit"}},"outcome":"succeeded"}})
                } else if path.ends_with("/active") {json!({"data":{}})}
                else if path.contains("/message/") {json!({"data":{"type":"user","text":"task"}})}
                else {json!({"data":[]})}).into_response()
            }
        });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let reg = dir.path().join("registration");
        std::fs::write(
            &reg,
            json!({"url":endpoint,"password":"fixture"}).to_string(),
        )
        .unwrap();
        let command: RunnerCommand = serde_json::from_value(json!({"id":id,"runner_id":runner,"stream_id":stream,"ownership_epoch":1,"kind":"run_agent","state":"accepted","created_unix_ms":1,
            "agent_spec":{"prompt":"task","profile":{"provider_id":"openai","model_id":"explicit","toolchain":"","start_command":"","test_command":""}}})).unwrap();
        let journal =
            crate::journal::Journal::open(&dir.path().join("db"), "http://factory.invalid", runner)
                .unwrap();
        let record = journal.receive(command).unwrap();
        assert!(record.recovery_required);
        let args = Args::try_parse_from([
            "factory-runner",
            "--id",
            &runner.to_string(),
            "--name",
            "fixture",
            "--workspace-root",
            root.to_str().unwrap(),
            "--opencode-registration",
            reg.to_str().unwrap(),
        ])
        .unwrap();
        let observed = observe(&record, &args).await;
        assert_eq!(observed.phase, AgentPhase::Unknown);
        assert!(observed.summary.contains("prompt_projected=true"));
        assert!(observed.summary.contains("outcome=succeeded"));
        assert!(record.agent.is_none() && record.result.is_none());
        mismatch.store(true, Ordering::SeqCst);
        assert!(observe(&record, &args)
            .await
            .summary
            .contains("identity differs"));
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        server.abort();
    }
}
