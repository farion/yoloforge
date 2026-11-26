use super::*;

pub(super) fn request_clarification(
    tx: &Transaction<'_>, ticket: &Ticket, controller: &RunnerCommand, stage: &str, text: &str,
) -> Result<()> {
    let source = ticket.execution_command.ok_or(StoreError::Internal)?;
    let id = QuestionId::new();
    let value = Question {
        id, command_id: controller.id, session_id: controller.session_id(), update_seq: Revision(1),
        external: ExternalQuestion {
            external_id: format!("intent-clarification-{id}"),
            kind: QuestionKind::Form,
            payload: serde_json::json!({
                "title": format!("{} · Rückfrage ({stage})", ticket.key),
                "ticket_id": ticket.id, "source_command_id": source, "stage": stage,
                "fields": [{"key":"response","title":text,"type":"string","required":true}]
            }),
        },
        state: QuestionState::Pending, answer: None, answer_request: None,
        orchestration_note: String::new(),
    };
    tx.execute("INSERT INTO questions(id,command_id,kind,external_id,data) VALUES(?1,?2,'form',?3,?4)",
        params![id.to_string(), controller.id.to_string(), value.external.external_id, serde_json::to_string(&value)?])?;
    event(tx, "question.created", id.to_string(), &value)
}

pub(super) fn clarification_answer(
    tx: &Transaction<'_>, ticket: &Ticket, stage: &str,
) -> Result<Option<serde_json::Value>> {
    let raw: Option<String> = tx.query_row(
        "SELECT data FROM questions WHERE external_id LIKE 'intent-clarification-%'
         AND json_extract(data,'$.external.payload.ticket_id')=?1
         AND json_extract(data,'$.external.payload.source_command_id')=?2
         AND json_extract(data,'$.external.payload.stage')=?3
         AND json_extract(data,'$.state')='answered' ORDER BY rowid DESC LIMIT 1",
        params![ticket.id.to_string(), ticket.execution_command.map(|id| id.to_string()), stage],
        |r| r.get(0),
    ).optional()?;
    raw.map(|raw| -> Result<_> {
        let q: Question = serde_json::from_str(&raw)?;
        Ok(serde_json::json!({"question":q.external.payload["fields"][0]["title"],"answer":q.answer}))
    }).transpose()
}

pub(super) fn close_intent_clarifications(tx: &Transaction<'_>, ticket: TicketId) -> Result<()> {
    let ids = tx.prepare(
        "SELECT id FROM questions WHERE external_id LIKE 'intent-clarification-%'
         AND json_extract(data,'$.external.payload.ticket_id')=?1
         AND json_extract(data,'$.state')='pending'",
    )?.query_map([ticket.to_string()], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        let mut question = load(tx, id.parse().map_err(|_| StoreError::Internal)?)?;
        question.state = QuestionState::Unknown;
        save(tx, &mut question)?;
    }
    Ok(())
}

pub(super) fn valid_clarification(text: &str) -> bool {
    !text.trim().is_empty() && text.len() <= 1024 && !text.chars().any(char::is_control)
}

pub(super) fn clarification_text(body: &AnswerQuestion) -> Result<&str> {
    body.answer["response"].as_str().map(str::trim).filter(|text|
        body.answer.as_object().is_some_and(|fields| fields.len() == 1)
        && !text.is_empty() && text.len() <= 4000
        && !text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    ).ok_or(StoreError::Invalid("Eine Antwort mit maximal 4000 UTF-8-Bytes ist erforderlich"))
}

fn answer_intent_clarification(tx: &Transaction<'_>, value: &mut Question, body: &AnswerQuestion) -> Result<()> {
    let payload = &value.external.payload;
    let raw: String = tx.query_row(
        "SELECT t.data FROM tickets t JOIN active_projects p ON t.project_id=p.id
         WHERE t.id=?1 AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)",
        [payload["ticket_id"].as_str()], |r| r.get(0),
    ).optional()?.ok_or(StoreError::WorkConflict)?;
    let mut ticket: Ticket = serde_json::from_str(&raw)?;
    let source = ticket.execution_command.ok_or(StoreError::WorkConflict)?;
    if value.state != QuestionState::Pending || ticket.draft || ticket.status != TicketState::Blocked
        || payload["source_command_id"] != source.to_string()
    {
        return Err(StoreError::WorkConflict);
    }
    let text = clarification_text(body)?;
    match payload["stage"].as_str() {
        Some("Review") => super::validation::resume_after_clarification(tx, &mut ticket, source, value.command_id)?,
        Some("Recovery") => super::workspace_recovery::resume_after_clarification(tx, &mut ticket, source, value.command_id)?,
        _ => return Err(StoreError::WorkConflict),
    }
    value.answer = Some(serde_json::json!({"response":text}));
    value.answer_request = Some(body.request_id);
    value.state = QuestionState::Answered;
    save(tx, value)
}

pub(super) fn save(tx: &Transaction<'_>, question: &mut Question) -> Result<()> {
    question.update_seq.0 = question
        .update_seq
        .0
        .checked_add(1)
        .ok_or(StoreError::Internal)?;
    tx.execute(
        "UPDATE questions SET data=?1 WHERE id=?2",
        params![serde_json::to_string(question)?, question.id.to_string()],
    )?;
    event(tx, "question.updated", question.id.to_string(), question)
}
pub(super) fn load(tx: &Transaction<'_>, id: QuestionId) -> Result<Question> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM questions WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?)
}
fn list(tx: &Transaction<'_>, id: CommandId) -> Result<Vec<Question>> {
    let mut stmt = tx.prepare("SELECT data FROM questions WHERE command_id=?1 ORDER BY rowid")?;
    let rows = stmt.query_map([id.to_string()], |r| r.get::<_, String>(0))?;
    let mut values = Vec::new();
    for row in rows {
        values.push(serde_json::from_str(&row?)?);
    }
    Ok(values)
}
pub(super) fn close(tx: &Transaction<'_>, id: CommandId) -> Result<()> {
    for mut value in list(tx, id)? {
        if !matches!(
            value.state,
            QuestionState::Answered | QuestionState::Unknown
        ) {
            value.state = QuestionState::Unknown;
            save(tx, &mut value)?;
        }
    }
    Ok(())
}
impl Store {
    pub fn sync_questions(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: QuestionSnapshot,
    ) -> Result<Vec<Question>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let command = super::work::command(&tx, id, runner, body.ownership_epoch)?;
        if super::tasks::deletion_stop_requested(&tx, id)? {
            return Ok(Vec::new());
        }
        if command.kind != CommandKind::RunAgent || command.state != CommandState::Accepted {
            return Err(StoreError::WorkConflict);
        }
        if body.questions.len() > 64 || serde_json::to_vec(&body.questions)?.len() > 131072 {
            return Err(StoreError::Invalid("Question snapshot too large"));
        }
        let mut seen = std::collections::HashSet::new();
        let mut existing = list(&tx, id)?;
        let mut can_check = None;
        for external in &body.questions {
            let kind = match external.kind {
                QuestionKind::Form => "form",
                QuestionKind::Permission => "permission",
            };
            if external.external_id.is_empty()
                || external.external_id.len() > 200
                || !external
                    .external_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || external.payload["id"] != external.external_id
                || external.payload["sessionID"] != command.session_id()
                || !seen.insert((kind, external.external_id.clone()))
            {
                return Err(StoreError::Invalid(
                    "Invalid or duplicate external question",
                ));
            }
            match external.kind {
                QuestionKind::Form if !external.payload["fields"].is_array() => {
                    return Err(StoreError::Invalid("Form fields missing"))
                }
                QuestionKind::Permission
                    if !external.payload["action"].is_string()
                        || !external.payload["resources"].is_array() =>
                {
                    return Err(StoreError::Invalid("Permission details missing"))
                }
                _ => {}
            }
            if let Some(value) = existing.iter_mut().find(|q| {
                q.external.kind == external.kind && q.external.external_id == external.external_id
            }) {
                if value.external != *external
                    && !matches!(value.state, QuestionState::Answered | QuestionState::Unknown)
                {
                    value.state = QuestionState::Unknown;
                    save(&tx, value)?;
                }
            } else {
                let try_automatic = match can_check {
                    Some(enabled) => enabled,
                    None => {
                        let enabled = super::question_orchestrator::can_check(&tx, &command)?;
                        can_check = Some(enabled);
                        enabled
                    }
                };
                let mut value = Question {
                    orchestration_note: if try_automatic && external.kind == QuestionKind::Permission {
                        QUESTION_AUTOMATION_PENDING.into()
                    } else { String::new() },
                    id: QuestionId::new(),
                    command_id: id,
                    session_id: command.session_id(),
                    update_seq: Revision(1),
                    external: external.clone(),
                    state: QuestionState::Pending,
                    answer: None,
                    answer_request: None,
                };
                tx.execute("INSERT INTO questions(id,command_id,kind,external_id,data) VALUES(?1,?2,?3,?4,?5)",params![value.id.to_string(),id.to_string(),kind,external.external_id,serde_json::to_string(&value)?])?;
                let sequence = append_event(&tx, "question.created", &value.id.to_string(), &value)?;
                if try_automatic && external.kind == QuestionKind::Form {
                    super::question_orchestrator::try_documented_form(&tx, &value)?;
                    value = load(&tx, value.id)?;
                }
                super::desktop::record_notification(&tx, sequence, "question.created",
                    &value.id.to_string(), &serde_json::to_value(&value)?)?;
                if let Some(intent) = super::directs::intent_request(&tx, &command, &value)? {
                    if !intent.enqueue {
                        super::directs::save_intent(&tx, &command, &value, &intent)?;
                        value.answer = Some(serde_json::json!({"decision":"once"}));
                        value.answer_request = Some(RequestId::new());
                        value.state = QuestionState::AnswerQueued;
                        save(&tx, &mut value)?;
                    }
                }
                existing.push(value);
            }
        }
        for value in &mut existing {
            if !body.questions.iter().any(|q| {
                q.kind == value.external.kind && q.external_id == value.external.external_id
            }) && matches!(
                value.state,
                QuestionState::Pending | QuestionState::AnswerQueued | QuestionState::Rejected
            ) {
                value.state = QuestionState::Unknown;
                save(&tx, value)?;
            }
        }
        tx.commit()?;
        if can_check == Some(true) {
            self.wake_scheduler();
        }
        Ok(existing)
    }

    pub fn answer_question(&self, id: QuestionId, body: AnswerQuestion) -> Result<Question> {
        let result = self.mutate(
            body.request_id,
            &format!("question.answer/{id}"),
            &body,
            |tx| Self::queue_answer(tx, id, &body),
        );
        if result.as_ref().is_ok_and(|q| q.external.external_id.starts_with("routing-clarification-")
            || q.external.external_id.starts_with("intent-clarification-")
            || (factory_protocol::directs::DirectIntent::from_permission(&q.external)
            .is_some_and(|intent| intent.enqueue) && body.answer["decision"] == "once"))
        {
            self.wake_scheduler();
        }
        result
    }

    pub(super) fn queue_answer(
        tx: &Transaction<'_>,
        id: QuestionId,
        body: &AnswerQuestion,
    ) -> Result<Question> {
        let mut value = load(tx, id)?;
        if value.external.external_id.starts_with("intent-clarification-") {
            answer_intent_clarification(tx, &mut value, body)?;
            return Ok(value);
        }
        if value.external.external_id.starts_with("routing-clarification-")
            && super::routing::answer_clarification(tx, &mut value, body)?
        {
            return Ok(value);
        }
        let data: String = tx.query_row(
            "SELECT data FROM commands WHERE id=?1",
            [value.command_id.to_string()],
            |r| r.get(0),
        )?;
        let command: RunnerCommand = serde_json::from_str(&data)?;
        if super::tasks::deletion_stop_requested(tx, command.id)? || super::controls::load(tx, command.stream_id)?
            .is_some_and(|c| c.state != PauseState::Released)
        {
            return Err(StoreError::WorkConflict);
        }
        if command.state != CommandState::Accepted
            || command
                .agent_progress
                .as_ref()
                .is_some_and(|p| matches!(p.phase, AgentPhase::Unknown | AgentPhase::Finalizing))
            || !matches!(
                value.state,
                QuestionState::Pending | QuestionState::Rejected
            )
        {
            return Err(StoreError::WorkConflict);
        }
        if !body.answer.is_object() || serde_json::to_vec(&body.answer)?.len() > 32768 {
            return Err(StoreError::Invalid("Invalid answer object"));
        }
        if value.external.kind == QuestionKind::Permission
            && (body.answer.as_object().unwrap().len() != 1
                || !matches!(
                    body.answer["decision"].as_str(),
                    Some("once" | "always" | "reject")
                ))
        {
            return Err(StoreError::Invalid(
                "Permission decision must be once, always or reject",
            ));
        }
        if let Some(intent) = super::directs::intent_request(tx, &command, &value)? {
            if body.answer["decision"] == "always" {
                return Err(StoreError::Invalid("Each Direct Intent requires individual approval"));
            }
            if body.answer["decision"] == "once" {
                super::directs::save_intent(tx, &command, &value, &intent)?;
            }
        }
        value.answer = Some(body.answer.clone());
        value.answer_request = Some(body.request_id);
        value.state = QuestionState::AnswerQueued;
        save(tx, &mut value)?;
        Ok(value)
    }

    pub fn answer_delivery(
        &self,
        runner: RunnerId,
        id: QuestionId,
        body: AnswerDelivery,
    ) -> Result<Question> {
        self.mutate(
            body.request_id,
            &format!("question.delivery/{runner}/{id}"),
            &body,
            |tx| {
                let mut value = load(tx, id)?;
                super::work::command(tx, value.command_id, runner, body.ownership_epoch)?;
                if super::tasks::deletion_stop_requested(tx, value.command_id)? {
                    return Ok(value);
                }
                if value.answer_request != Some(body.answer_request)
                    || !matches!(
                        body.state,
                        QuestionState::Answered | QuestionState::Unknown | QuestionState::Rejected
                    )
                {
                    return Err(StoreError::WorkConflict);
                }
                if value.state == body.state {
                    return Ok(value);
                }
                if !matches!(
                    value.state,
                    QuestionState::AnswerQueued | QuestionState::Unknown
                ) {
                    return Err(StoreError::WorkConflict);
                }
                value.state = body.state;
                save(tx, &mut value)?;
                Ok(value)
            },
        )
    }
}
