use super::*;
pub(super) fn terminate(tx: &Transaction<'_>, id: CommandId) -> Result<()> {
    if let Some(mut v) = latest(tx, id)?.filter(|v| !v.state.terminal()) {
        v.state = CompactionState::Aborted;
        v.revision.0 += 1;
        v.summary = "Runtime removed; native compaction outcome not established".into();
        tx.execute(
            "UPDATE compactions SET data=?1 WHERE request_id=?2",
            params![serde_json::to_string(&v)?, v.request_id.to_string()],
        )?;
        event(tx, "session.compaction", id.to_string(), &v)?;
    }
    Ok(())
}
pub(super) fn latest(tx: &Transaction<'_>, id: CommandId) -> Result<Option<Compaction>> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM compactions WHERE command_id=?1 ORDER BY rowid DESC LIMIT 1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    data.map(|v| serde_json::from_str(&v).map_err(Into::into))
        .transpose()
}
impl Store {
    pub fn request_compaction(&self, id: CommandId, body: RequestCompaction) -> Result<Compaction> {
        self.mutate(
            body.request_id,
            &format!("command.compact/{id}"),
            &body,
            |tx| Self::request_compaction_in(tx, id, &body),
        )
    }
    pub(super) fn request_compaction_in(
        tx: &Transaction<'_>,
        id: CommandId,
        body: &RequestCompaction,
    ) -> Result<Compaction> {
        line(&body.reason, 512)?;
        let data: Option<String> = tx
            .query_row(
                "SELECT data FROM commands WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let command: RunnerCommand = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
        let mut stream = super::work::stream(tx, command.stream_id)?;
        if stream.revision != body.expected_stream_revision {
            return Err(StoreError::RevisionConflict(stream.revision));
        }
        if command.kind != CommandKind::RunAgent
            || command.state != CommandState::Accepted
            || command.agent_progress.is_none()
            || latest(tx, id)?.is_some_and(|v| !v.state.terminal())
            || super::controls::load(tx, stream.id)?
                .is_some_and(|v| v.state != PauseState::Released)
        {
            return Err(StoreError::WorkConflict);
        }
        let archived:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM archive_status WHERE command_id=?1 AND json_extract(data,'$.final_scan_complete')=1)",[id.to_string()],|r|r.get(0))?;
        if archived {
            return Err(StoreError::WorkConflict);
        }
        let value = Compaction {
            request_id: body.request_id,
            command_id: id,
            session_id: command.session_id(),
            message_id: compaction_message_id(body.request_id),
            reason: body.reason.clone(),
            revision: Revision(1),
            state: CompactionState::Requested,
            summary: "Awaiting owning runner".into(),
        };
        tx.execute(
            "INSERT INTO compactions(request_id,command_id,data) VALUES(?1,?2,?3)",
            params![
                body.request_id.to_string(),
                id.to_string(),
                serde_json::to_string(&value)?
            ],
        )?;
        event(tx, "session.compaction", id.to_string(), &value)?;
        let status = stream.status;
        super::work::save_stream(tx, &mut stream, status)?;
        Ok(value)
    }
    pub fn compaction(&self, id: CommandId) -> Result<Option<Compaction>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands WHERE id=?1)",
            [id.to_string()],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound);
        }
        latest(&tx, id)
    }
    pub fn runner_compaction(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
    ) -> Result<Option<Compaction>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        super::work::command(&tx, id, runner, epoch)?;
        latest(&tx, id)
    }
    pub fn report_compaction(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: ReportCompaction,
    ) -> Result<Compaction> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let command = super::work::command(&tx, id, runner, body.ownership_epoch)?;
        let mut value = latest(&tx, id)?.ok_or(StoreError::NotFound)?;
        if value.request_id != body.request_id
            || command.state != CommandState::Accepted
            || matches!(
                body.state,
                CompactionState::Requested | CompactionState::Aborted
            )
        {
            return Err(StoreError::WorkConflict);
        }
        line(&body.summary, 240)?;
        if value.state == body.state && value.summary == body.summary {
            return Ok(value);
        }
        if value.state.terminal()
            || (matches!(
                value.state,
                CompactionState::Running | CompactionState::Admitted
            ) && body.state == CompactionState::Rejected)
            || (value.state == CompactionState::Running && body.state == CompactionState::Admitted)
        {
            return Err(StoreError::WorkConflict);
        }
        value.state = body.state;
        value.summary = body.summary;
        value.revision.0 += 1;
        tx.execute(
            "UPDATE compactions SET data=?1 WHERE request_id=?2",
            params![serde_json::to_string(&value)?, value.request_id.to_string()],
        )?;
        event(&tx, "session.compaction", id.to_string(), &value)?;
        tx.commit()?;
        Ok(value)
    }
}
