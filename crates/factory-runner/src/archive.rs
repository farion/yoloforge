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
    revision: u64,
    next: Option<String>,
    final_scan: bool,
    finished: bool,
    pending: Option<ArchiveBatch>,
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
        return Ok(true);
    }
    if record.archive.pending.is_some() {
        upload(factory, journal, record, registration).await?;
        return Ok(record.archive.finished);
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
    let session_id = agent_session_id(record.command.id);
    let session = client
        .session(&session_id)
        .await?
        .context("Archive session missing")?;
    ensure!(
        session["id"] == session_id
            && session["metadata"]["factory_command"] == record.command.id.to_string()
            && session["location"]["directory"] == execution.workspace.path,
        "Archive session identity changed"
    );
    let (messages, next) = client
        .archive_page(&session_id, record.archive.next.as_deref())
        .await?;
    let revision = record
        .archive
        .revision
        .checked_add(1)
        .context("Archive revision exhausted")?;
    let captured_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;
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
    Ok(record.archive.finished || !final_result)
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
