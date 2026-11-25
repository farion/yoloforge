use crate::{
    journal::{Journal, Record},
    Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_opencode::OpenCodeClient;
use factory_protocol::{client::FactoryClient, ReportArchive};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Cursor {
    #[serde(default)]
    pub(crate) native_archived: bool,
    #[serde(default)]
    pub(crate) blobs: Vec<crate::archive_blobs::Pending>,
    #[serde(default)]
    pub(crate) supplement_scan_complete: bool,
    #[serde(default)]
    pub(crate) supplement_after: u64,
    #[serde(default)]
    retry_after_ms: u64,
    #[serde(default)]
    backfill_failures: u8,
    #[serde(default)]
    backfill_error: String,
    pub(crate) revision: u64,
    next: Option<String>,
    final_scan: bool,
    pub(crate) finished: bool,
    pending: Option<ArchiveBatch>,
}
/// Repair historical, already reported executions in idle runner slots only.
/// Existing final archives remain immutable; the normal outbox handles uncertain uploads.
pub async fn backfill(
    factory: &FactoryClient,
    journal: &Journal,
    args: &Args,
    registration: RegistrationId,
) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;
    let Some(mut record) = tokio::task::block_in_place(|| journal.archive_backfill(now))? else {
        return Ok(());
    };
    let outcome = match crate::setup::effective(args, &record.command) {
        Ok(effective) => match step(factory, journal, &mut record, &effective, registration).await {
            Ok(done) => archive_decision(journal, &mut record, &effective).await.map(|_| done),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    match outcome {
        Ok(_) => {
            record.archive.retry_after_ms = 0;
            record.archive.backfill_failures = 0;
            record.archive.backfill_error.clear();
        }
        Err(error) => {
            record.archive.backfill_failures = record.archive.backfill_failures.saturating_add(1);
            record.archive.retry_after_ms =
                now.saturating_add(60_000u64 << record.archive.backfill_failures.min(5));
            record.archive.backfill_error = error.to_string().chars().take(240).collect();
            eprintln!(
                "Archive backfill deferred for {}: {}",
                record.command.id, record.archive.backfill_error
            );
        }
    }
    tokio::task::block_in_place(|| journal.save(&record))?;
    Ok(())
}
/// Only one-shot, read-only controller runs are disposable. Worker sessions
/// remain available for follow-ups. Require both durable receipts before cleanup.
pub async fn archive_decision(journal: &Journal, record: &mut Record, args: &Args) -> Result<()> {
    if record.archive.native_archived
        || !record.reported
        || record.result.is_none()
        || !record.archive.finished
        || record.archive.blobs.iter().any(|b| b.offset < b.file.bytes)
        || !record.command.agent_spec.as_ref().is_some_and(|spec| {
            spec.read_only
                && matches!(
                    spec.role.as_str(),
                    "orchestrator" | "orchestrator-question" | "orchestrator-memory" | "supervisor" | "memorizer" | "reviewer"
                )
        })
    {
        return Ok(());
    }
    let execution = record.agent.as_ref().context("Decision execution missing")?;
    let client = OpenCodeClient::from_registration(
        args.opencode_registration.as_ref().context("Decision registration missing")?,
    )?;
    ensure!(execution.endpoint == client.endpoint(), "Decision endpoint changed");
    let id = record.command.session_id();
    if let Some(session) = client.session(&id).await? {
        ensure!(
            session["id"] == id
                && session["metadata"]["factory_command"] == record.command.session_root().to_string()
                && session["location"]["directory"] == execution.workspace.path,
            "Decision session identity changed"
        );
        ensure!(!client.active(&id).await?, "Decision session still active");
        for kind in ["inbox", "form", "permission"] {
            ensure!(
                client.session_items(&id, kind).await?.as_array().is_some_and(|v| v.is_empty()),
                "Decision session has pending input"
            );
        }
        if session["time"]["archived"].as_u64().is_none_or(|v| v == 0) {
            client.archive_session(&id).await?;
        }
        ensure!(
            client.session(&id).await?
                .is_some_and(|s| s["time"]["archived"].as_u64().is_some_and(|v| v > 0)),
            "Decision session archive state not confirmed"
        );
    }
    record.archive.native_archived = true;
    tokio::task::block_in_place(|| journal.save(record))?;
    Ok(())
}
/// One bounded page per step. A terminal result is held until the final traversal has been uploaded.
pub async fn step(
    factory: &FactoryClient,
    journal: &Journal,
    record: &mut Record,
    args: &Args,
    registration: RegistrationId,
) -> Result<bool> {
    if record.command.kind != CommandKind::RunAgent
        || record.agent.as_ref().is_none_or(|a| !a.prompt_attempted)
    {
        return Ok(true);
    }
    if record.archive.finished {
        // Supplemental reconstruction is idle work after the result is reported.
        // It reads Factory's immutable archive, so OpenCode may already be gone.
        if record.reported && !record.archive.supplement_scan_complete {
            let page = factory
                .runner_archive(
                    record.command.runner_id,
                    registration,
                    record.command.id,
                    record.command.ownership_epoch,
                    record.archive.supplement_after,
                    record.archive.revision,
                )
                .await?;
            if page.entries.is_empty() {
                record.archive.supplement_scan_complete = true;
            } else {
                ensure!(
                    page.next_after > record.archive.supplement_after,
                    "Archive source cursor did not advance"
                );
                let messages = page
                    .entries
                    .into_iter()
                    .map(|e| e.message)
                    .collect::<Vec<_>>();
                let revision = record.archive.revision;
                tokio::task::block_in_place(|| {
                    crate::archive_blobs::capture(record, args, &messages, revision)
                })?;
                record.archive.supplement_after = page.next_after;
            }
            tokio::task::block_in_place(|| journal.save(record))?;
        }
        return crate::archive_blobs::step(factory, journal, record, registration).await;
    }
    if record.archive.pending.is_some() {
        upload(factory, journal, record, registration).await?;
        return Ok(record.archive.finished
            && record
                .archive
                .blobs
                .iter()
                .all(|b| b.offset == b.file.bytes));
    }
    if !crate::archive_blobs::step(factory, journal, record, registration).await? {
        return Ok(false);
    }
    let final_result = record.result.is_some();
    if final_result && !record.archive.final_scan {
        record.archive.final_scan = true;
        record.archive.next = None;
        tokio::task::block_in_place(|| journal.save(record))?;
    }
    let execution = record.agent.as_ref().context("Archive execution missing")?;
    let client = OpenCodeClient::from_registration(
        args.opencode_registration
            .as_ref()
            .context("Archive registration missing")?,
    )?;
    ensure!(
        execution.endpoint == client.endpoint(),
        "Archive endpoint changed"
    );
    let session_id = record.command.session_id();
    let session = client
        .session(&session_id)
        .await?
        .context("Archive session missing")?;
    ensure!(
        session["id"] == session_id
            && session["metadata"]["factory_command"] == record.command.session_root().to_string()
            && session["location"]["directory"] == execution.workspace.path,
        "Archive session identity changed"
    );
    let (mut messages, next) = client
        .archive_page(&session_id, record.archive.next.as_deref())
        .await?;
    tokio::task::block_in_place(|| {
        crate::archive_files::capture(&mut messages, &execution.workspace.path)
    })?;
    let revision = record
        .archive
        .revision
        .checked_add(1)
        .context("Archive revision exhausted")?;
    let captured_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;
    tokio::task::block_in_place(|| {
        crate::archive_blobs::capture(record, args, &messages, revision)
    })?;
    let batch = ArchiveBatch {
        session_id,
        revision,
        captured_unix_ms,
        final_scan_complete: record.archive.final_scan && next.is_none(),
        messages,
    };
    record.archive.next = next;
    record.archive.pending = Some(batch);
    tokio::task::block_in_place(|| journal.save(record))?;
    upload(factory, journal, record, registration).await?;
    Ok((record.archive.finished
        && record
            .archive
            .blobs
            .iter()
            .all(|b| b.offset == b.file.bytes))
        || !final_result)
}
async fn upload(
    factory: &FactoryClient,
    journal: &Journal,
    record: &mut Record,
    registration: RegistrationId,
) -> Result<()> {
    let batch = record
        .archive
        .pending
        .as_ref()
        .context("Missing archive outbox")?
        .clone();
    let status = factory
        .report_archive(
            record.command.runner_id,
            registration,
            record.command.id,
            &ReportArchive {
                ownership_epoch: record.command.ownership_epoch,
                batch: batch.clone(),
            },
        )
        .await?;
    ensure!(
        status.revision == batch.revision
            && status.session_id == batch.session_id
            && status.final_scan_complete == batch.final_scan_complete,
        "Archive confirmation differs"
    );
    record.archive.revision = batch.revision;
    record.archive.finished = batch.final_scan_complete;
    record.archive.pending = None;
    tokio::task::block_in_place(|| journal.save(record))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::State,
        http::StatusCode,
        response::{IntoResponse, Response},
        routing::put,
        Json, Router,
    };
    use std::sync::{Arc, Mutex};
    #[tokio::test(flavor = "multi_thread")]
    async fn decision_cleanup_requires_receipts_idle_identity_and_survives_restart() {
        use axum::{extract::Path, routing::get};
        use clap::Parser;
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let runner = RunnerId::new();
        let id = CommandId::new();
        let session_id = agent_session_id(id);
        let active = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let updates = Arc::new(Mutex::new(Vec::new()));
        let active_state = active.clone();
        let active_id = session_id.clone();
        let seen = updates.clone();
        let observed_updates = updates.clone();
        let router = Router::new()
            .route("/api/session/active", get(move || {
                let active = active_state.clone();
                let id = active_id.clone();
                async move {
                    Json(json!({"data":if active.load(std::sync::atomic::Ordering::SeqCst) {json!({id:{}})} else {json!({})}}))
                }
            }))
            .route("/api/session/{id}", get(move |Path(sid): Path<String>| {
                let updates = observed_updates.clone();
                async move {
                Json(json!({"data":{"id":sid,"metadata":{"factory_command":id.to_string()},"location":{"directory":"/work"},"time":{"archived":if updates.lock().unwrap().is_empty(){0}else{42}}}}))
                }
            }).patch(move |Json(body): Json<serde_json::Value>| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(body);
                    StatusCode::NO_CONTENT
                }
            }))
            .route("/api/session/{id}/{kind}", get(|| async { Json(json!({"data":[]})) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let registration = dir.path().join("service.json");
        std::fs::write(&registration, json!({"url":endpoint,"password":"test"}).to_string()).unwrap();
        let args = Args::try_parse_from([
            "factory-runner", "--server", "http://127.0.0.1:1",
            "--id", &runner.to_string(), "--name", "test",
            "--opencode-registration", registration.to_str().unwrap(),
        ]).unwrap();
        let path = dir.path().join("journal");
        let journal = Journal::open(&path, "http://127.0.0.1:1", runner).unwrap();
        let command: RunnerCommand = serde_json::from_value(json!({
            "id":id,"stream_id":StreamId::new(),"runner_id":runner,"ownership_epoch":1,
            "kind":"run_agent","state":"accepted","created_unix_ms":1,
            "agent_spec":{"prompt":"Extract observations","profile":ExecutionProfile::default(),"read_only":true,"role":"memorizer"}
        })).unwrap();
        let mut record = journal.receive(command).unwrap();
        record.agent = Some(serde_json::from_value(json!({
            "workspace":{"path":"/work","branch":"main","base_commit":"a".repeat(40)},
            "endpoint":endpoint,"session_attempted":true,"prompt_attempted":true
        })).unwrap());
        record.result = Some(CommandOutput::Agent {agent: AgentProgress {
            session_id, message_id:agent_message_id(id), phase:AgentPhase::Succeeded, summary:"new".into(),
        }});
        // No archive or result receipt: do not contact or mutate OpenCode.
        archive_decision(&journal, &mut record, &args).await.unwrap();
        record.archive.finished = true;
        archive_decision(&journal, &mut record, &args).await.unwrap();
        record.reported = true;
        record.agent.as_mut().unwrap().workspace.path = "/other".into();
        assert!(archive_decision(&journal, &mut record, &args).await.is_err());
        record.agent.as_mut().unwrap().workspace.path = "/work".into();
        assert!(archive_decision(&journal, &mut record, &args).await.is_err());
        assert!(updates.lock().unwrap().is_empty());
        active.store(false, std::sync::atomic::Ordering::SeqCst);
        // Workers are reusable and must not be archived.
        record.command.agent_spec.as_mut().unwrap().read_only = false;
        archive_decision(&journal, &mut record, &args).await.unwrap();
        assert!(updates.lock().unwrap().is_empty());
        record.command.agent_spec.as_mut().unwrap().read_only = true;
        record.archive.supplement_scan_complete = true;
        journal.save(&record).unwrap();
        assert!(journal.archive_backfill(0).unwrap().is_some(), "Historical decisions must be selected");
        archive_decision(&journal, &mut record, &args).await.unwrap();
        drop(journal);
        let journal = Journal::open(&path, "http://127.0.0.1:1", runner).unwrap();
        let mut record = journal.record(id).unwrap().unwrap();
        // Simulate losing the local receipt after the native PATCH succeeded.
        record.archive.native_archived = false;
        archive_decision(&journal, &mut record, &args).await.unwrap();
        archive_decision(&journal, &mut record, &args).await.unwrap();
        assert!(record.archive.native_archived);
        assert!(journal.archive_backfill(u64::MAX).unwrap().is_none());
        assert_eq!(*updates.lock().unwrap(), vec![json!({"archived":true})]);
        server.abort();
    }
    async fn receive(
        State(seen): State<Arc<Mutex<Vec<ReportArchive>>>>,
        Json(body): Json<ReportArchive>,
    ) -> Response {
        let mut seen = seen.lock().unwrap();
        seen.push(body.clone());
        if seen.len() == 1 {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        Json(ArchiveStatus {
            session_id: body.batch.session_id,
            revision: body.batch.revision,
            captured_unix_ms: body.batch.captured_unix_ms,
            final_scan_complete: body.batch.final_scan_complete,
        })
        .into_response()
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn historical_backfill_replays_outbox_after_restart_and_never_reexecutes_work() {
        use clap::Parser;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/v1/runners/{runner}/commands/{id}/archive", put(receive).get(|| async {Json(serde_json::json!({"status":null,"entries":[],"next_after":0,"termination":null}))}))
            .with_state(seen.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = FactoryClient::new(&endpoint, "test-archive-token-0123456789abcdef").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let runner = RunnerId::new();
        let id = CommandId::new();
        let command:RunnerCommand=serde_json::from_value(serde_json::json!({"id":id,"stream_id":StreamId::new(),"runner_id":runner,"ownership_epoch":1,"kind":"run_agent","state":"accepted","created_unix_ms":1})).unwrap();
        let args = Args::try_parse_from([
            "factory-runner",
            "--id",
            &runner.to_string(),
            "--name",
            "test",
        ])
        .unwrap();
        let journal = Journal::open(&path, &endpoint, runner).unwrap();
        let mut record = journal.receive(command).unwrap();
        record.reported = true;
        record.result = Some(CommandOutput::Agent {
            agent: AgentProgress {
                session_id: record.command.session_id(),
                message_id: agent_message_id(id),
                phase: AgentPhase::Succeeded,
                summary: "original result".into(),
            },
        });
        record.agent=Some(serde_json::from_value(serde_json::json!({"workspace":{"path":"/work","branch":"main","base_commit":"a".repeat(40)},"endpoint":"http://never-contacted","session_attempted":true,"prompt_attempted":true})).unwrap());
        record.archive.pending = Some(ArchiveBatch {
            session_id: record.command.session_id(),
            revision: 1,
            captured_unix_ms: 42,
            final_scan_complete: true,
            messages: vec![serde_json::json!({"id":"msg_old","text":"historical content"})],
        });
        journal.save(&record).unwrap();
        let result = record.result.clone();
        backfill(&client, &journal, &args, RegistrationId::new())
            .await
            .unwrap();
        assert!(journal.archive_backfill(0).unwrap().is_none());
        drop(journal);
        let journal = Journal::open(&path, &endpoint, runner).unwrap();
        let mut saved = journal.record(id).unwrap().unwrap();
        assert_eq!(saved.archive.backfill_failures, 1);
        assert!(saved.archive.pending.is_some());
        saved.archive.retry_after_ms = 0;
        journal.save(&saved).unwrap();
        backfill(&client, &journal, &args, RegistrationId::new())
            .await
            .unwrap();
        let saved = journal.record(id).unwrap().unwrap();
        assert!(saved.archive.finished);
        assert_eq!(saved.result, result);
        assert!(saved.reported);
        backfill(&client, &journal, &args, RegistrationId::new())
            .await
            .unwrap();
        assert!(
            journal
                .record(id)
                .unwrap()
                .unwrap()
                .archive
                .supplement_scan_complete
        );
        assert!(journal.archive_backfill(u64::MAX).unwrap().is_none());
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].batch, requests[1].batch);
        task.abort();
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn restart_replays_identical_archive_outbox_before_advancing() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/v1/runners/{runner}/commands/{id}/archive", put(receive))
            .with_state(seen.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = FactoryClient::new(&endpoint, "test-archive-token-0123456789abcdef").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let runner = RunnerId::new();
        let command = RunnerCommand {
            id: CommandId::new(),
            stream_id: StreamId::new(),
            runner_id: runner,
            ownership_epoch: 1,
            kind: CommandKind::RunAgent,
            state: CommandState::Accepted,
            result: None,
            created_unix_ms: 1,
            workspace_spec: None,
            agent_spec: None,
            agent_progress: None,
        };
        let journal = Journal::open(&path, &endpoint, runner).unwrap();
        let mut record = journal.receive(command.clone()).unwrap();
        record.archive.pending = Some(ArchiveBatch {
            session_id: agent_session_id(command.id),
            revision: 1,
            captured_unix_ms: 42,
            final_scan_complete: true,
            messages: vec![serde_json::json!({"id":"msg_test","text":"untruncated"})],
        });
        journal.save(&record).unwrap();
        assert!(
            upload(&client, &journal, &mut record, RegistrationId::new())
                .await
                .is_err()
        );
        drop(journal);
        let journal = Journal::open(&path, &endpoint, runner).unwrap();
        let mut record = journal.receive(command.clone()).unwrap();
        assert!(record.archive.pending.is_some());
        assert_eq!(record.archive.revision, 0);
        upload(&client, &journal, &mut record, RegistrationId::new())
            .await
            .unwrap();
        drop(journal);
        let journal = Journal::open(&path, &endpoint, runner).unwrap();
        let record = journal.receive(command).unwrap();
        assert!(record.archive.finished);
        assert!(record.archive.pending.is_none());
        assert_eq!(record.archive.revision, 1);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].batch, seen[1].batch);
        task.abort();
    }
}
