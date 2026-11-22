use crate::{
    journal::{Journal, Record},
    Args,
};
use anyhow::Result;
use factory_core::*;
use factory_opencode::OpenCodeClient;
use factory_protocol::{client::FactoryClient, *};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    question: Question,
    delivery_request: RequestId,
    state: Option<QuestionState>,
    reported: bool,
}

async fn reconcile(api: &OpenCodeClient, receipt: &Receipt) -> QuestionState {
    let q = &receipt.question;
    if q.external.kind == QuestionKind::Form {
        if let Ok(Some(detail)) = api
            .question_detail(&q.session_id, q.external.kind, &q.external.external_id)
            .await
        {
            let mut original = detail.clone();
            if let Some(object) = original.as_object_mut() {
                object.remove("state");
            }
            if detail["state"]["status"] == "answered"
                && original == q.external.payload
                && q.answer.as_ref() == detail["state"].get("answer")
            {
                return QuestionState::Answered;
            }
        }
    }
    // A disappeared permission or still-pending form does not prove delivery.
    QuestionState::Unknown
}

pub async fn process(
    client: &FactoryClient,
    journal: &Journal,
    record: &mut Record,
    args: &Args,
    registration: RegistrationId,
) -> Result<()> {
    crate::containment::ready(record, args).await?;
    let mut questions = Vec::new();
    if let Some(snapshot) = &record.question_snapshot {
        questions = client
            .sync_questions(
                args.id,
                registration,
                record.command.id,
                &QuestionSnapshot {
                    ownership_epoch: record.command.ownership_epoch,
                    questions: snapshot.clone(),
                },
            )
            .await?;
    }
    let Some(path) = args.opencode_registration.as_ref() else {
        return Ok(());
    };
    let Ok(api) = OpenCodeClient::from_registration(path) else {
        return Ok(());
    };
    if record
        .agent
        .as_ref()
        .is_none_or(|e| e.endpoint != api.endpoint())
    {
        return Ok(());
    }
    // First flush known outcomes even if the question disappeared after reply.
    let mut selected = record
        .replies
        .iter()
        .find(|(_, r)| !r.reported)
        .map(|(id, _)| *id);
    if selected.is_none() {
        selected = questions
            .iter()
            .find(|q| q.state == QuestionState::AnswerQueued)
            .and_then(|q| q.answer_request);
    }
    if selected.is_none() {
        selected = record
            .replies
            .iter()
            .find(|(_, r)| r.state == Some(QuestionState::Unknown))
            .map(|(id, _)| *id);
    }
    let Some(answer_id) = selected else {
        return Ok(());
    };
    if let std::collections::btree_map::Entry::Vacant(entry) = record.replies.entry(answer_id) {
        let Some(question) = questions
            .iter()
            .find(|q| q.answer_request == Some(answer_id))
            .cloned()
        else {
            return Ok(());
        };
        let Some(answer) = question.answer.as_ref() else {
            return Ok(());
        };
        // Confirm the exact current pending request and a live execution before
        // persisting intent and replying. Never recreate a vanished question.
        let resource = match question.external.kind {
            QuestionKind::Form => "form",
            QuestionKind::Permission => "permission",
        };
        let pending = match api.session_items(&question.session_id, resource).await {
            Ok(value) => value,
            Err(_) => return Ok(()), // no external write attempted; retry the read later
        };
        let current = Some(pending).and_then(|v| {
            v.as_array().and_then(|v| {
                v.iter()
                    .find(|p| p["id"] == question.external.external_id)
                    .cloned()
            })
        });
        let active = match api.active(&question.session_id).await {
            Ok(active) => active,
            Err(_) => return Ok(()),
        };
        let valid = current.as_ref() == Some(&question.external.payload) && active;
        let mut receipt = Receipt {
            question: question.clone(),
            delivery_request: RequestId::new(),
            state: None,
            reported: false,
        };
        if !valid {
            receipt.state = Some(QuestionState::Unknown);
        }
        entry.insert(receipt);
        tokio::task::block_in_place(|| journal.save(record))?;
        if valid {
            let state = api
                .reply_question(
                    &question.session_id,
                    question.external.kind,
                    &question.external.external_id,
                    answer,
                )
                .await
                .unwrap_or(QuestionState::Unknown);
            record.replies.get_mut(&answer_id).unwrap().state = Some(state);
            tokio::task::block_in_place(|| journal.save(record))?;
        }
    } else {
        let receipt = record.replies.get(&answer_id).unwrap();
        if matches!(receipt.state, None | Some(QuestionState::Unknown)) {
            let observed = reconcile(&api, receipt).await;
            let receipt = record.replies.get_mut(&answer_id).unwrap();
            if receipt.state != Some(observed) {
                receipt.state = Some(observed);
                receipt.reported = false;
                receipt.delivery_request = RequestId::new();
                tokio::task::block_in_place(|| journal.save(record))?;
            }
        }
    }
    let receipt = record.replies.get(&answer_id).unwrap();
    if !receipt.reported {
        client
            .answer_delivery(
                args.id,
                registration,
                receipt.question.id,
                &AnswerDelivery {
                    request_id: receipt.delivery_request,
                    ownership_epoch: record.command.ownership_epoch,
                    answer_request: answer_id,
                    state: receipt.state.unwrap_or(QuestionState::Unknown),
                },
            )
            .await?;
        record.replies.get_mut(&answer_id).unwrap().reported = true;
        tokio::task::block_in_place(|| journal.save(record))?;
    }
    Ok(())
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
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    #[derive(Clone)]
    struct Fake {
        question: Arc<Mutex<Question>>,
        posts: Arc<AtomicUsize>,
    }
    async fn handler(State(fake): State<Fake>, request: Request) -> axum::response::Response {
        let path = request.uri().path().to_string();
        if path.ends_with("/delivery") {
            let bytes = axum::body::to_bytes(request.into_body(), 65536)
                .await
                .unwrap();
            let delivery: AnswerDelivery = serde_json::from_slice(&bytes).unwrap();
            let mut q = fake.question.lock().unwrap();
            q.state = delivery.state;
            return Json(q.clone()).into_response();
        }
        let q = fake.question.lock().unwrap().clone();
        if path.ends_with("/questions") {
            return Json(vec![q]).into_response();
        }
        if path.ends_with("/reply") {
            fake.posts.fetch_add(1, Ordering::SeqCst);
            // Simulate acceptance followed by an unconfirmed HTTP response.
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        if path == "/api/session/active" {
            return Json(json!({"data":{q.session_id:{"type":"running"}}})).into_response();
        }
        if path.ends_with("/form") {
            return Json(json!({"data":[q.external.payload]})).into_response();
        }
        let mut detail = q.external.payload;
        detail["state"] = json!({"status":"answered","answer":q.answer});
        Json(json!({"data":detail})).into_response()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lost_reply_confirmation_is_reconciled_after_restart_without_a_second_post() {
        let dir = tempfile::tempdir().unwrap();
        let runner = RunnerId::new();
        let command_id = CommandId::new();
        let session = agent_session_id(command_id);
        let answer_request = RequestId::new();
        let question = Question {
            orchestration_note: String::new(),
            id: QuestionId::new(),
            command_id,
            session_id: session.clone(),
            revision: Revision(2),
            state: QuestionState::AnswerQueued,
            external: ExternalQuestion {
                external_id: "form_test".into(),
                kind: QuestionKind::Form,
                payload: json!({"id":"form_test","sessionID":session,"title":"test","fields":[]}),
            },
            answer: Some(json!({"token":"known"})),
            answer_request: Some(answer_request),
        };
        let fake = Fake {
            question: Arc::new(Mutex::new(question.clone())),
            posts: Arc::new(AtomicUsize::new(0)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().fallback(handler).with_state(fake.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let reg = dir.path().join("registration.json");
        std::fs::write(
            &reg,
            serde_json::to_vec(&json!({"url":endpoint,"password":"test"})).unwrap(),
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
            opencode_registration: Some(reg),
            journal: None,
            repositories: None,
            workspace_root: None,
        };
        let command = RunnerCommand {
            id: command_id,
            stream_id: StreamId::new(),
            runner_id: runner,
            ownership_epoch: 1,
            kind: CommandKind::RunAgent,
            state: CommandState::Accepted,
            result: None,
            workspace_spec: None,
            agent_spec: None,
            agent_progress: None,
            created_unix_ms: 1,
        };
        let path = dir.path().join("journal.db");
        let journal = Journal::open(&path, &endpoint, runner).unwrap();
        let mut record = journal.receive(command.clone()).unwrap();
        record.agent=Some(serde_json::from_value(json!({"workspace":{"path":"/work","branch":"test","base_commit":"a".repeat(40)},"endpoint":endpoint,"session_attempted":true,"prompt_attempted":true})).unwrap());
        record.question_snapshot = Some(vec![question.external]);
        let client =
            FactoryClient::new(&endpoint, "test-only-credential-0123456789abcdef").unwrap();
        let registration = RegistrationId::new();
        process(&client, &journal, &mut record, &args, registration)
            .await
            .unwrap();
        assert_eq!(fake.question.lock().unwrap().state, QuestionState::Unknown);
        assert_eq!(fake.posts.load(Ordering::SeqCst), 1);
        drop(journal);
        let journal = Journal::open(&path, &endpoint, runner).unwrap();
        let mut record = journal.receive(command).unwrap();
        process(&client, &journal, &mut record, &args, registration)
            .await
            .unwrap();
        assert_eq!(fake.question.lock().unwrap().state, QuestionState::Answered);
        assert_eq!(fake.posts.load(Ordering::SeqCst), 1);
        server.abort();
    }
}
