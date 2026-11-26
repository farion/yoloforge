//! An operator-authorized discard is fenced before the Runner touches its files.
use super::*;

pub(super) fn retired(tx: &Connection, stream: StreamId) -> Result<bool> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM draft_resets WHERE stream_id=?1)",
        [stream.to_string()], |row| row.get(0))?)
}

pub(super) fn require_not_resetting(tx: &Connection, ticket: TicketId) -> Result<()> {
    let pending = tx.query_row("SELECT EXISTS(SELECT 1 FROM draft_resets WHERE ticket_id=?1 AND completed=0)",
        [ticket.to_string()], |row| row.get::<_, bool>(0))?;
    if pending { return Err(StoreError::WorkConflict); }
    Ok(())
}

fn load_ticket(tx: &Transaction<'_>, id: TicketId) -> Result<Ticket> {
    let raw: String = tx.query_row(
        "SELECT t.data FROM tickets t JOIN active_projects p ON p.id=t.project_id
         WHERE t.id=?1 AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)",
        [id.to_string()], |row| row.get(0),
    ).optional()?.ok_or(StoreError::NotFound)?;
    Ok(serde_json::from_str(&raw)?)
}

impl Store {
    pub fn return_intent_to_draft(&self, id: TicketId, body: ReturnIntentToDraft) -> Result<DraftReset> {
        let result = self.mutate(body.request_id, &format!("task.return_to_draft/{id}"), &body, |tx| {
            let mut ticket = load_ticket(tx, id)?;
            if body.expected_revision != ticket.revision {
                return Err(StoreError::RevisionConflict(ticket.revision));
            }
            if ticket.draft || !matches!(ticket.status, TicketState::InProgress | TicketState::Blocked)
                || ticket.execution_command != Some(body.previous_command) {
                return Err(StoreError::WorkConflict);
            }
            require_not_resetting(tx, id)?;
            super::tasks::require_no_publication_recovery(tx, ticket.project_id)?;
            let raw: String = tx.query_row("SELECT data FROM commands WHERE id=?1", [body.previous_command.to_string()], |row| row.get(0))?;
            let command: RunnerCommand = serde_json::from_str(&raw)?;
            let mut owner = super::work::stream(tx, command.stream_id)?;
            let root = command.session_root();
            if command.kind != CommandKind::RunAgent
                || command.requires_target_git()
                || command.agent_spec.as_ref().is_none_or(|spec| spec.read_only || !spec.role.is_empty())
                || !matches!(&command.result, Some(CommandOutput::Agent { agent }) if agent.phase == AgentPhase::Failed
                    || (agent.phase == AgentPhase::Succeeded && command_publication_blocked(&command)))
                || !matches!(command.state, CommandState::Failed | CommandState::Succeeded)
                || owner.project_id != ticket.project_id || !owner.ticket_ids.contains(&ticket.id)
                || owner.runner_id != command.runner_id || owner.ownership_epoch != command.ownership_epoch
                || retired(tx, owner.id)?
                || owner.checkpoint.is_some()
                || super::validation::awaiting(tx, owner.id)?
                || super::termination::active(tx, owner.id)?
                || super::controls::load(tx, owner.id)?.is_some_and(|c| c.state != PauseState::Released)
            {
                return Err(StoreError::WorkConflict);
            }
            if !tx.query_row("SELECT id=?1 FROM commands WHERE stream_id=?2 ORDER BY rowid DESC LIMIT 1",
                params![command.id.to_string(), owner.id.to_string()], |row| row.get::<_, bool>(0))?
                || !tx.query_row("SELECT EXISTS(SELECT 1 FROM archive_status WHERE command_id=?1
                    AND json_extract(data,'$.final_scan_complete')=1)",
                    [command.id.to_string()], |row| row.get::<_, bool>(0))?
                || tx.query_row("SELECT EXISTS(SELECT 1 FROM commands WHERE stream_id=?1 AND state IN ('pending','accepted'))",
                    [owner.id.to_string()], |row| row.get::<_, bool>(0))?
                || tx.query_row("SELECT EXISTS(SELECT 1 FROM questions WHERE command_id=?1
                    AND json_extract(data,'$.state') IN ('pending','rejected','answer_queued'))",
                    [command.id.to_string()], |row| row.get::<_, bool>(0))?
            {
                return Err(StoreError::WorkConflict);
            }
            let related = {
                let mut query = tx.prepare("SELECT t.data FROM tickets t WHERE t.project_id=?1 AND t.id<>?2
                    AND (json_extract(t.data,'$.requested_session')=?3
                        OR EXISTS(SELECT 1 FROM stream_tickets st WHERE st.ticket_id=t.id AND st.stream_id=?4))
                    AND json_extract(t.data,'$.status') NOT IN ('done','cancelled')
                    AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)")?;
                let rows = query.query_map(params![ticket.project_id.to_string(), id.to_string(), root.to_string(), owner.id.to_string()],
                    |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for raw in related {
                let mut other: Ticket = serde_json::from_str(&raw)?;
                if other.execution_command.is_some() || !matches!(other.status, TicketState::Backlog | TicketState::Ready) {
                    return Err(StoreError::Invalid("Eine andere Ausführung nutzt den Worktree noch"));
                }
                other.requested_session = None;
                other.session_prompt_request = None;
                other.routing_note = if other.draft { "Entwurf — noch nicht freigegeben" }
                    else { "Die bisherige Session wird verworfen; Orchestrator ordnet den Intent neu zu." }.into();
                detach(tx, &mut owner, other.id)?;
                super::scheduler::save_ticket(tx, &mut other)?;
            }
            let reset = DraftReset {
                request_id: body.request_id, ticket_id: id, command_id: command.id, root_id: root,
                stream_id: owner.id, ownership_epoch: owner.ownership_epoch,
            };
            tx.execute("INSERT INTO draft_resets(request_id,ticket_id,stream_id,root_id,runner_id,data) VALUES(?1,?2,?3,?4,?5,?6)",
                params![body.request_id.to_string(), id.to_string(), owner.id.to_string(), root.to_string(),
                    owner.runner_id.to_string(), serde_json::to_string(&reset)?])?;
            let closure = SessionClosure {
                request_id: body.request_id, root_command_id: root, last_command_id: command.id,
                session_id: command.session_id(), reason: DISCARDED_SESSION_REASON.into(),
                closed_unix_ms: super::super::unix_ms(),
            };
            tx.execute("INSERT INTO session_closures(root_command_id,data) VALUES(?1,?2)
                ON CONFLICT(root_command_id) DO UPDATE SET data=excluded.data",
                params![root.to_string(), serde_json::to_string(&closure)?])?;
            ticket.status = TicketState::Blocked;
            ticket.routing_note = "Zurück zum Entwurf: Runner muss Session und Worktree noch verwerfen.".into();
            super::scheduler::save_ticket(tx, &mut ticket)?;
            super::work::save_stream(tx, &mut owner, StreamState::Blocked)?;
            event(tx, "intent.draft_reset_requested", id.to_string(), &reset)?;
            Ok(reset)
        });
        if result.is_ok() { self.wake_scheduler(); }
        result
    }

    pub fn pending_draft_reset(&self, runner: RunnerId) -> Result<Option<DraftReset>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let raw: Option<String> = db.query_row(
            "SELECT d.data FROM draft_resets d JOIN streams s ON s.id=d.stream_id
             WHERE d.runner_id=?1 AND d.completed=0 AND json_extract(s.data,'$.runner_id')=?1
               AND json_extract(s.data,'$.ownership_epoch')=json_extract(d.data,'$.ownership_epoch')
             ORDER BY d.rowid LIMIT 1",
            [runner.to_string()], |row| row.get(0),
        ).optional()?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into)).transpose()
    }

    pub fn confirm_draft_reset(&self, runner: RunnerId, request: RequestId) -> Result<Ticket> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (raw, completed): (String, bool) = tx.query_row(
            "SELECT data,completed FROM draft_resets WHERE request_id=?1 AND runner_id=?2",
            params![request.to_string(), runner.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?.ok_or(StoreError::NotFound)?;
        let reset: DraftReset = serde_json::from_str(&raw)?;
        let mut ticket = load_ticket(&tx, reset.ticket_id)?;
        if !completed {
            let mut owner = super::work::stream(&tx, reset.stream_id)?;
            if owner.runner_id != runner || owner.ownership_epoch != reset.ownership_epoch
                || ticket.execution_command != Some(reset.command_id) || ticket.status != TicketState::Blocked {
                return Err(StoreError::WorkConflict);
            }
            detach(&tx, &mut owner, ticket.id)?;
            super::work::save_stream(&tx, &mut owner, StreamState::Blocked)?;
            ticket.status = TicketState::Backlog;
            ticket.draft = true;
            ticket.execution_command = None;
            ticket.requested_session = None;
            ticket.session_prompt_request = None;
            ticket.routing_note = "Entwurf — noch nicht freigegeben".into();
            super::scheduler::save_ticket(&tx, &mut ticket)?;
            tx.execute("UPDATE draft_resets SET completed=1 WHERE request_id=?1", [request.to_string()])?;
            event(&tx, "intent.returned_to_draft", ticket.id.to_string(), &ticket)?;
        }
        tx.commit()?;
        self.wake_scheduler();
        Ok(ticket)
    }
}

fn detach(tx: &Transaction<'_>, owner: &mut Workstream, ticket: TicketId) -> Result<()> {
    owner.ticket_ids.retain(|id| *id != ticket);
    tx.execute("DELETE FROM stream_tickets WHERE stream_id=?1 AND ticket_id=?2",
        params![owner.id.to_string(), ticket.to_string()])?;
    for table in ["scheduler_runs", "task_routing", "task_edits"] {
        tx.execute(&format!("DELETE FROM {table} WHERE ticket_id=?1"), [ticket.to_string()])?;
    }
    Ok(())
}
