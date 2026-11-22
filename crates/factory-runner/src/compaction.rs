use crate::{
    journal::{Journal, Record},
    Args,
};
use anyhow::{ensure, Context, Result};
use factory_core::*;
use factory_opencode::OpenCodeClient;
use factory_protocol::{client::FactoryClient, ReportCompaction};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    request: Compaction,
    attempted: bool,
    state: CompactionState,
}
fn summary(state: CompactionState) -> &'static str {
    match state {
        CompactionState::Requested => "Awaiting compaction admission",
        CompactionState::Admitted => "Compaction admitted; completion not yet verified",
        CompactionState::Running => "Native compaction is running",
        CompactionState::Succeeded => "Native compaction completed; original session retained",
        CompactionState::Failed => "Native compaction failed; inspect archived session",
        CompactionState::Aborted => "Runtime removed; native compaction outcome not established",
        CompactionState::Rejected => {
            "Compaction not submitted or definitively rejected; no automatic retry"
        }
        CompactionState::Unknown => {
            "Compaction delivery or result uncertain; read-only reconciliation, no resubmission"
        }
    }
}
/// A read failure is not an empty inbox. External writes are attempted at most once per request.
pub async fn process(
    factory: &FactoryClient,
    journal: &Journal,
    record: &mut Record,
    args: &Args,
    registration: RegistrationId,
    allow_new: bool,
) -> Result<bool> {
    crate::containment::ready(record, args).await?;
    if record.command.kind != CommandKind::RunAgent {
        return Ok(true);
    }
    let request = factory
        .runner_compaction(
            record.command.runner_id,
            registration,
            record.command.id,
            record.command.ownership_epoch,
        )
        .await?;
    let Some(request) = request.filter(|r| !r.state.terminal()) else {
        return Ok(true);
    };
    ensure!(
        request.command_id == record.command.id
            && request.session_id == record.command.session_id()
            && request.message_id == compaction_message_id(request.request_id),
        "Compaction request identity differs"
    );
    let mut receipt = record
        .compactions
        .get(&request.request_id)
        .cloned()
        .unwrap_or(Receipt {
            request: request.clone(),
            attempted: false,
            state: CompactionState::Requested,
        });
    ensure!(
        receipt.request.reason == request.reason
            && receipt.request.message_id == request.message_id
            && receipt.request.command_id == request.command_id,
        "Compaction request changed"
    );
    if !receipt.state.terminal() {
        if !receipt.attempted && !allow_new {
            receipt.state = CompactionState::Rejected;
        } else {
            receipt.state = match advance(
                factory,
                journal,
                record,
                args,
                registration,
                &mut receipt,
                allow_new,
            )
            .await
            {
                Ok(state) => state,
                Err(_) => CompactionState::Unknown,
            };
        }
        record
            .compactions
            .insert(request.request_id, receipt.clone());
        tokio::task::block_in_place(|| journal.save(record))?;
    }
    let observed = factory
        .report_compaction(
            record.command.runner_id,
            registration,
            record.command.id,
            &ReportCompaction {
                request_id: request.request_id,
                ownership_epoch: record.command.ownership_epoch,
                state: receipt.state,
                summary: summary(receipt.state).into(),
            },
        )
        .await?;
    Ok(observed.state.terminal())
}
async fn advance(
    factory: &FactoryClient,
    journal: &Journal,
    record: &mut Record,
    args: &Args,
    registration: RegistrationId,
    receipt: &mut Receipt,
    allow_new: bool,
) -> Result<CompactionState> {
    let execution = record.agent.as_ref().context("No session execution")?;
    ensure!(execution.prompt_attempted, "No agent prompt admitted yet");
    let client = OpenCodeClient::from_registration(
        args.opencode_registration
            .as_ref()
            .context("No OpenCode registration")?,
    )?;
    ensure!(
        execution.endpoint == client.endpoint(),
        "Compaction endpoint changed"
    );
    let session = client
        .session(&receipt.request.session_id)
        .await?
        .context("Compaction session missing")?;
    ensure!(
        session["id"] == receipt.request.session_id
            && session["metadata"]["factory_command"] == record.command.session_root().to_string()
            && session["location"]["directory"] == execution.workspace.path,
        "Compaction session identity changed"
    );
    let observed = client
        .compaction_state(&receipt.request.session_id, &receipt.request.message_id)
        .await?;
    if observed != CompactionState::Unknown {
        receipt.attempted = true;
        return Ok(observed);
    }
    if receipt.attempted {
        return Ok(observed);
    }
    ensure!(allow_new, "Compaction admission no longer allowed");
    let held = factory
        .runner_control(
            record.command.runner_id,
            registration,
            record.command.id,
            record.command.ownership_epoch,
        )
        .await?
        .is_some_and(|c| c.state != PauseState::Released);
    if held {
        return Ok(CompactionState::Rejected);
    }
    receipt.attempted = true;
    receipt.state = CompactionState::Unknown;
    record
        .compactions
        .insert(receipt.request.request_id, receipt.clone());
    tokio::task::block_in_place(|| journal.save(record))?;
    client
        .compact(&receipt.request.session_id, &receipt.request.message_id)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::State,
        http::StatusCode,
        response::{IntoResponse, Response},
        routing::{get, post},
        Json, Router,
    };
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    };
    #[derive(Clone)]
    struct Fake {
        request: Arc<Mutex<Compaction>>,
        posts: Arc<AtomicUsize>,
        visible: Arc<AtomicBool>,
    }
    async fn current(State(f): State<Fake>) -> Json<Compaction> {
        Json(f.request.lock().unwrap().clone())
    }
    async fn report(State(f): State<Fake>, Json(body): Json<ReportCompaction>) -> Json<Compaction> {
        let mut v = f.request.lock().unwrap();
        v.state = body.state;
        v.summary = body.summary;
        v.revision.0 += 1;
        Json(v.clone())
    }
    async fn session(State(f): State<Fake>) -> Json<serde_json::Value> {
        let v = f.request.lock().unwrap();
        Json(
            serde_json::json!({"data":{"id":v.session_id,"metadata":{"factory_command":v.command_id},"location":{"directory":"/prepared"}}}),
        )
    }
    async fn message(State(f): State<Fake>) -> Response {
        if !f.visible.load(Ordering::SeqCst) {
            return StatusCode::NOT_FOUND.into_response();
        }
        let v = f.request.lock().unwrap();
        Json(serde_json::json!({"data":{"id":v.message_id,"type":"compaction","reason":"manual","status":"completed"}})).into_response()
    }
    async fn compact(State(f): State<Fake>, Json(body): Json<serde_json::Value>) -> Response {
        assert_eq!(body["id"], f.request.lock().unwrap().message_id);
        assert_eq!(body["delivery"], "steer");
        f.posts.fetch_add(1, Ordering::SeqCst);
        StatusCode::SERVICE_UNAVAILABLE.into_response()
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn lost_compaction_confirmation_is_reconciled_after_restart_without_reposting() {
        let runner = RunnerId::new();
        let id = CommandId::new();
        let request_id = RequestId::new();
        let request = Compaction {
            request_id,
            command_id: id,
            session_id: agent_session_id(id),
            message_id: compaction_message_id(request_id),
            reason: "obsolete context".into(),
            revision: Revision(1),
            state: CompactionState::Requested,
            summary: "requested".into(),
        };
        let fake = Fake {
            request: Arc::new(Mutex::new(request)),
            posts: Arc::new(AtomicUsize::new(0)),
            visible: Arc::new(AtomicBool::new(false)),
        };
        let router = Router::new()
            .route(
                "/v1/runners/{runner}/commands/{id}/compaction",
                get(current).put(report),
            )
            .route(
                "/v1/runners/{runner}/commands/{id}/control",
                get(|| async { Json(Option::<StreamControl>::None) }),
            )
            .route("/api/session/{session}", get(session))
            .route("/api/session/{session}/message/{id}", get(message))
            .route(
                "/api/session/{session}/inbox",
                get(|| async { Json(serde_json::json!({"data":[]})) }),
            )
            .route("/api/session/{session}/compact", post(compact))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        let registration = dir.path().join("service.json");
        std::fs::write(
            &registration,
            serde_json::to_vec(&serde_json::json!({"url":endpoint,"password":"test-only"}))
                .unwrap(),
        )
        .unwrap();
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
            server: endpoint.clone(),
            id: runner,
            name: "test".into(),
            location: crate::Location::Local,
            opencode_registration: Some(registration),
            journal: Some(path.clone()),
            repositories: None,
            workspace_root: None,
        };
        let factory =
            FactoryClient::new(&endpoint, "test-only-factory-token-0123456789abcdef").unwrap();
        let command = RunnerCommand {
            id,
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
        record.agent=Some(serde_json::from_value(serde_json::json!({"workspace":{"path":"/prepared","branch":"test","base_commit":"a".repeat(40)},"endpoint":endpoint,"session_attempted":true,"prompt_attempted":true})).unwrap());
        journal.save(&record).unwrap();
        assert!(!process(
            &factory,
            &journal,
            &mut record,
            &args,
            RegistrationId::new(),
            true
        )
        .await
        .unwrap());
        assert_eq!(fake.posts.load(Ordering::SeqCst), 1);
        drop(journal);
        let journal = Journal::open(&path, &endpoint, runner).unwrap();
        let mut record = journal.receive(command).unwrap();
        assert!(record.compactions[&request_id].attempted);
        assert!(!process(
            &factory,
            &journal,
            &mut record,
            &args,
            RegistrationId::new(),
            true
        )
        .await
        .unwrap());
        assert_eq!(fake.posts.load(Ordering::SeqCst), 1);
        fake.visible.store(true, Ordering::SeqCst);
        assert!(process(
            &factory,
            &journal,
            &mut record,
            &args,
            RegistrationId::new(),
            false
        )
        .await
        .unwrap());
        assert_eq!(fake.posts.load(Ordering::SeqCst), 1);
        assert_eq!(
            fake.request.lock().unwrap().state,
            CompactionState::Succeeded
        );
        // A new request arriving after the execution result must never start work.
        {
            let mut request = fake.request.lock().unwrap();
            request.request_id = RequestId::new();
            request.message_id = compaction_message_id(request.request_id);
            request.state = CompactionState::Requested;
        }
        assert!(process(
            &factory,
            &journal,
            &mut record,
            &args,
            RegistrationId::new(),
            false
        )
        .await
        .unwrap());
        assert_eq!(
            fake.request.lock().unwrap().state,
            CompactionState::Rejected
        );
        assert_eq!(fake.posts.load(Ordering::SeqCst), 1);
        task.abort();
    }
}
