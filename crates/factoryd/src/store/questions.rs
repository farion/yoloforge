use super::*;

pub(super) fn save(tx: &Transaction<'_>, question: &mut Question) -> Result<()> {
    question.revision.0 = question
        .revision
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
        if command.kind != CommandKind::RunAgent || command.state != CommandState::Accepted {
            return Err(StoreError::WorkConflict);
        }
        if body.questions.len() > 64 || serde_json::to_vec(&body.questions)?.len() > 131072 {
            return Err(StoreError::Invalid("Question snapshot too large"));
        }
        let mut seen = std::collections::HashSet::new();
        let mut existing = list(&tx, id)?;
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
                if value.external != *external && value.state != QuestionState::Unknown {
                    value.state = QuestionState::Unknown;
                    save(&tx, value)?;
                }
            } else {
                let value = Question {
                    orchestration_note: String::new(),
                    id: QuestionId::new(),
                    command_id: id,
                    session_id: command.session_id(),
                    revision: Revision(1),
                    external: external.clone(),
                    state: QuestionState::Pending,
                    answer: None,
                    answer_request: None,
                };
                tx.execute("INSERT INTO questions(id,command_id,kind,external_id,data) VALUES(?1,?2,?3,?4,?5)",params![value.id.to_string(),id.to_string(),kind,external.external_id,serde_json::to_string(&value)?])?;
                event(&tx, "question.created", value.id.to_string(), &value)?;
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
        Ok(existing)
    }

    pub fn answer_question(&self, id: QuestionId, body: AnswerQuestion) -> Result<Question> {
        self.mutate(
            body.request_id,
            &format!("question.answer/{id}"),
            &body,
            |tx| Self::queue_answer(tx, id, &body),
        )
    }

    pub(super) fn queue_answer(
        tx: &Transaction<'_>,
        id: QuestionId,
        body: &AnswerQuestion,
    ) -> Result<Question> {
        let mut value = load(tx, id)?;
        if value.revision != body.expected_revision {
            return Err(StoreError::RevisionConflict(value.revision));
        }
        let data: String = tx.query_row(
            "SELECT data FROM commands WHERE id=?1",
            [value.command_id.to_string()],
            |r| r.get(0),
        )?;
        let command: RunnerCommand = serde_json::from_str(&data)?;
        if super::controls::load(tx, command.stream_id)?
            .is_some_and(|c| c.state != PauseState::Released)
        {
            return Err(StoreError::WorkConflict);
        }
        if command.state != CommandState::Accepted
            || command
                .agent_progress
                .as_ref()
                .is_some_and(|p| p.phase == AgentPhase::Unknown)
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
