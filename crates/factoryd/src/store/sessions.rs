use super::*;
pub(super) fn closed(tx: &Transaction<'_>, root: CommandId) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_closures WHERE root_command_id=?1)",
        [root.to_string()],
        |r| r.get(0),
    )?)
}

/// Called only inside the successful prompt admission transaction. Invalid
/// prompts and replayed requests therefore never reopen a session.
pub(super) fn reopen_for_prompt(tx: &Transaction<'_>, root: CommandId, ticket: TicketId) -> Result<()> {
    if tx.execute("DELETE FROM session_closures WHERE root_command_id=?1", [root.to_string()])? == 0 {
        return Ok(());
    }
    let stream: String = tx.query_row("SELECT stream_id FROM commands WHERE id=?1", [root.to_string()], |r| r.get(0))?;
    let mut owner = super::work::stream(tx, stream.parse().map_err(|_| StoreError::Internal)?)?;
    let status = owner.status;
    super::work::save_stream(tx, &mut owner, status)?;
    event(tx, "session.reopened", root.to_string(), &serde_json::json!({
        "root_command_id": root, "ticket_id": ticket, "session_id": agent_session_id(root),
    }))?;
    Ok(())
}
impl Store {
    /// Retire future scheduling of a successfully completed session, never an interrupt.
    pub fn close_session(&self, id: CommandId, body: CloseSession) -> Result<SessionClosure> {
        self.mutate(
            body.request_id,
            &format!("session.close/{id}"),
            &body,
            |tx| Self::close_session_in(tx, id, &body),
        )
    }
    pub(super) fn close_session_in(
        tx: &Transaction<'_>,
        id: CommandId,
        body: &CloseSession,
    ) -> Result<SessionClosure> {
        line(&body.reason, 512)?;
        let data: Option<String> = tx
            .query_row(
                "SELECT data FROM commands WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let command: RunnerCommand = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
        if command.kind != CommandKind::RunAgent || command.state != CommandState::Succeeded {
            return Err(StoreError::WorkConflict);
        }
        let mut owner = super::work::stream(tx, command.stream_id)?;
        if owner.revision != body.expected_stream_revision {
            return Err(StoreError::RevisionConflict(owner.revision));
        }
        let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands WHERE stream_id=?1 AND state IN ('pending','accepted'))",[owner.id.to_string()],|r|r.get(0))?;
        let latest:String=tx.query_row("SELECT id FROM commands WHERE stream_id=?1 AND json_extract(data,'$.kind')='run_agent' ORDER BY rowid DESC LIMIT 1",[owner.id.to_string()],|r|r.get(0))?;
        let archived:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM archive_status WHERE command_id=?1 AND json_extract(data,'$.final_scan_complete')=1)",[id.to_string()],|r|r.get(0))?;
        if latest != id.to_string()
            || busy
            || super::validation::awaiting(tx, owner.id)?
            || !archived
            || closed(tx, command.session_root())?
            || super::controls::load(tx, owner.id)?.is_some_and(|c| c.state != PauseState::Released)
        {
            return Err(StoreError::WorkConflict);
        }
        let closure = SessionClosure {
            request_id: body.request_id,
            root_command_id: command.session_root(),
            last_command_id: id,
            session_id: command.session_id(),
            reason: body.reason.clone(),
            closed_unix_ms: super::super::unix_ms(),
        };
        tx.execute(
            "INSERT INTO session_closures(root_command_id,data) VALUES(?1,?2)",
            params![
                closure.root_command_id.to_string(),
                serde_json::to_string(&closure)?
            ],
        )?;
        let status = owner.status;
        super::work::save_stream(tx, &mut owner, status)?;
        event(
            tx,
            "session.closed",
            closure.root_command_id.to_string(),
            &closure,
        )?;
        Ok(closure)
    }
    pub fn session_closure(&self, id: CommandId) -> Result<Option<SessionClosure>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let data: Option<String> = db
            .query_row(
                "SELECT data FROM commands WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let command: RunnerCommand = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
        let data: Option<String> = db
            .query_row(
                "SELECT data FROM session_closures WHERE root_command_id=?1",
                [command.session_root().to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }
}
