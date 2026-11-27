use super::*;
use factory_protocol::models::{ModelSelection, SubmitTaskWithModel};

fn delete_intent_in(tx: &Transaction<'_>, ticket: &mut Ticket) -> Result<()> {
    require_no_publication_recovery(tx, ticket.project_id)?;
    tombstone_intent(tx, ticket)
}

fn tombstone_intent(tx: &Transaction<'_>, ticket: &mut Ticket) -> Result<()> {
    super::draft_resets::require_not_resetting(tx, ticket.id)?;
    super::routing::close_clarification(tx, ticket.id)?;
    super::questions::close_intent_clarifications(tx, ticket.id)?;
    let id = ticket.id.to_string();
    ticket.status = TicketState::Cancelled;
    ticket.routing_note = "Intent gelöscht".into();
    super::scheduler::save_ticket(tx, ticket)?;
    tx.execute("INSERT INTO deleted_intents(ticket_id) VALUES(?1)", [&id])?;
    event(tx, "task.deleted", id, ticket)
}

pub(super) fn deletion_stop_requested(tx: &Connection, command: CommandId) -> Result<bool> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE aggregate_id=?1 AND kind='task.delete_stop_requested')",
        [command.to_string()], |row| row.get(0))?)
}

pub(super) fn stop_deleted_intent(tx: &Transaction<'_>, ticket: &Ticket) -> Result<()> {
    let rows = {
        let mut stmt = tx.prepare("SELECT c.data FROM commands c JOIN stream_tickets st ON st.stream_id=c.stream_id
            WHERE st.ticket_id=?1 AND c.state IN ('pending','accepted')
              AND (c.id=?2 OR ?2 IS NULL)
              AND NOT EXISTS(SELECT 1 FROM stream_tickets linked JOIN tickets t ON t.id=linked.ticket_id
                   WHERE linked.stream_id=c.stream_id AND t.id<>?1
                     AND json_extract(t.data,'$.status') NOT IN ('done','cancelled')
                     AND (json_extract(t.data,'$.execution_command')=c.id
                          OR json_extract(t.data,'$.execution_command') IS NULL))")?;
        let rows = stmt.query_map(params![ticket.id.to_string(), ticket.execution_command.map(|id| id.to_string())],
            |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for raw in rows {
        let mut command: RunnerCommand = serde_json::from_str(&raw)?;
        let mut owner = super::work::stream(tx, command.stream_id)?;
        if owner.runner_id != command.runner_id || owner.ownership_epoch != command.ownership_epoch {
            continue; // Do not stop an execution whose identity no longer matches.
        }
        event(tx, "task.delete_stop_requested", command.id.to_string(), ticket)?;
        super::questions::close(tx, command.id)?;
        if command.state == CommandState::Pending {
            command.state = CommandState::Failed;
            super::work::save_command(tx, &command, "failed")?;
            super::work::save_stream(tx, &mut owner, StreamState::Blocked)?;
        } else if command.kind == CommandKind::RunAgent {
            if !Store::terminate_deleted_runtime(tx, &command)? {
                super::controls::hold_deleted_intent(tx, &mut owner, command.id)?;
            }
        }
    }
    Ok(())
}

pub(super) fn require_no_publication_recovery(tx: &Connection, project: ProjectId) -> Result<()> {
    let busy = tx.query_row("SELECT EXISTS(SELECT 1 FROM publication_recovery_holds
        WHERE project_id=?1 AND expires_ms>?2)",
        params![project.to_string(), super::super::unix_ms()], |row| row.get::<_, bool>(0))?;
    if busy { return Err(StoreError::WorkConflict); }
    Ok(())
}

pub(super) fn block_unpublished_intents(
    tx: &Transaction<'_>, owner: &Workstream, command: CommandId, review_snapshot: bool,
) -> Result<()> {
    let note = if review_snapshot {
        SNAPSHOT_RETRY_NOTE
    } else {
        "Automatische Übernahme blockiert; Änderungen liegen im Arbeitsbereich."
    };
    set_publication_state(tx, owner, command, if review_snapshot { TicketState::InProgress } else { TicketState::Blocked }, note)
}

pub(super) fn set_publication_state(
    tx: &Transaction<'_>, owner: &Workstream, command: CommandId, state: TicketState, note: &str,
) -> Result<()> {
    for id in &owner.ticket_ids {
        let raw: String = tx.query_row("SELECT data FROM tickets WHERE id=?1",
            [id.to_string()], |row| row.get(0))?;
        let mut ticket: Ticket = serde_json::from_str(&raw)?;
        if ticket.execution_command != Some(command) || ticket.status == TicketState::Cancelled {
            continue;
        }
        ticket.status = state;
        ticket.revision.0 += 1;
        ticket.updated_unix_ms = super::super::unix_ms();
        ticket.routing_note = note.into();
        tx.execute("UPDATE tickets SET revision=?1,data=?2 WHERE id=?3",
            params![ticket.revision.0, serde_json::to_string(&ticket)?, id.to_string()])?;
        event(tx, "ticket.updated", id.to_string(), &ticket)?;
    }
    Ok(())
}

pub(super) fn recoverable_publication(tx: &Transaction<'_>, runner: RunnerId, id: CommandId, review_snapshot: bool) -> Result<Option<(RunnerCommand, Workstream)>> {
    let row: Option<(String, String, String)> = tx.query_row(
        "SELECT c.data,s.data,p.data FROM commands c JOIN streams s ON s.id=c.stream_id
         JOIN active_projects p ON p.id=s.project_id
         WHERE c.id=?1 AND c.runner_id=?2 AND c.state='succeeded'
            AND NOT EXISTS(SELECT 1 FROM draft_resets d WHERE d.stream_id=s.id)
           AND COALESCE(json_extract(p.data,'$.paused'),0)=0
            AND NOT EXISTS(SELECT 1 FROM commands n WHERE n.stream_id=c.stream_id AND n.rowid>c.rowid
                AND (n.state IN ('pending','accepted') OR json_extract(n.data,'$.kind')='run_agent' OR ?4=0))
           AND EXISTS(SELECT 1 FROM archive_status a WHERE a.command_id=
                 CASE WHEN json_extract(c.data,'$.agent_spec.role')=?3
                      THEN json_extract(c.data,'$.agent_spec.prompt') ELSE c.id END
                 AND json_extract(a.data,'$.final_scan_complete')=1)
         AND EXISTS(SELECT 1 FROM stream_tickets st JOIN tickets t ON t.id=st.ticket_id
                WHERE st.stream_id=c.stream_id AND json_extract(t.data,'$.execution_command')=c.id
                  AND json_extract(t.data,'$.status') IN ('in_progress','blocked')
                  AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id))
            AND NOT EXISTS(SELECT 1 FROM commands n JOIN streams w ON w.id=n.stream_id
                 WHERE n.state IN ('pending','accepted')
                   AND (n.stream_id=s.id OR (?4=0 AND w.project_id=s.project_id)))",
         params![id.to_string(), runner.to_string(), APPROVED_PUBLICATION_ROLE, review_snapshot],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;
    let Some((command, owner, project)) = row else { return Ok(None); };
    let command: RunnerCommand = serde_json::from_str(&command)?;
    let owner: Workstream = serde_json::from_str(&owner)?;
    let project: Project = serde_json::from_str(&project)?;
    if command.kind != CommandKind::RunAgent || command.requires_target_git()
        || owner.runner_id != runner || owner.ownership_epoch != command.ownership_epoch
        || !matches!(owner.status, StreamState::Blocked | StreamState::Review)
        || command.agent_spec.as_ref().is_none_or(|s| s.read_only || (!review_snapshot && s.defer_publication)
            || s.role == factory_protocol::directs::ROLE
             || (review_snapshot && !is_implementer(&command)))
        || command.workspace_spec.as_ref().is_none_or(|w| w.project_id != project.id
            || w.base_branch != project.base_branch
            || w.setup.as_ref().is_none_or(|setup| setup.runner_id != runner
                || !setup.repository_path.is_empty()
                || setup.repository_source != project.repository
                || !(super::work::absolute_runner_path(&setup.repository_source)
                    || (review_snapshot && setup.repository_source == "factory:managed"))))
        || !matches!(&command.result, Some(CommandOutput::Agent { agent })
            if agent.phase == AgentPhase::Succeeded
                && publication_was_blocked(&agent.summary)
                && !publication_was_confirmed(&agent.summary)
                && (review_snapshot || !agent.summary.starts_with(PUBLICATION_CONFLICT_PREFIX)))
    {
        return Ok(None);
    }
    if review_snapshot && !tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM scheduler_runs r JOIN tickets t ON t.id=r.ticket_id
         WHERE r.agent_id=?1 AND json_extract(t.data,'$.execution_command')=?1
          AND json_extract(t.data,'$.status') IN ('blocked','in_progress')
         AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)
         AND NOT EXISTS(SELECT 1 FROM validations v WHERE v.source_id=?1 AND v.retired=0))",
        [id.to_string()], |row| row.get::<_, bool>(0),
    )? {
        return Ok(None);
    }
    if review_snapshot && (super::sessions::closed(tx, command.session_root())?
        || super::termination::load(tx, command.id)?.is_some())
    {
        return Ok(None);
    }
    if !review_snapshot && is_implementer(&command)
        && tx.query_row("SELECT EXISTS(SELECT 1 FROM scheduler_runs WHERE agent_id=?1)",
            [id.to_string()], |row| row.get::<_, bool>(0))?
        && !tx.query_row("SELECT EXISTS(SELECT 1 FROM publication_recovery_holds
                WHERE command_id=?1 AND expires_ms>?2)",
            params![id.to_string(), super::super::unix_ms()], |row| row.get::<_, bool>(0))?
    {
        return Ok(None);
    }
    Ok(Some((command, owner)))
}
pub(super) fn editable(db: &Connection, t: &Ticket) -> Result<bool> {
    if t.execution_command.is_some()
        || !matches!(t.status, TicketState::Backlog | TicketState::Ready)
    {
        return Ok(false);
    }
    Ok(!db.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM active_projects WHERE id=?2)
        OR EXISTS(SELECT 1 FROM stream_tickets WHERE ticket_id=?1)
        OR EXISTS(SELECT 1 FROM scheduler_runs WHERE ticket_id=?1)
        OR EXISTS(SELECT 1 FROM task_routing WHERE ticket_id=?1)
        OR EXISTS(SELECT 1 FROM events WHERE aggregate_id=?1 AND kind='task.routing')",
        params![t.id.to_string(), t.project_id.to_string()],
        |r| r.get::<_, bool>(0),
    )?)
}
impl Store {
    pub fn delete_task(&self, id: TicketId, body: DeleteTask) -> Result<Ticket> {
        let result = self.mutate(body.request_id, &format!("task.delete/{id}"), &body, |tx| {
            let raw: Option<String> = tx.query_row("SELECT data FROM tickets WHERE id=?1",
                [id.to_string()], |row| row.get(0)).optional()?;
            let mut ticket: Ticket = serde_json::from_str(&raw.ok_or(StoreError::NotFound)?)?;
            let already_deleted = tx.query_row("SELECT EXISTS(SELECT 1 FROM deleted_intents WHERE ticket_id=?1)",
                [id.to_string()], |row| row.get::<_, bool>(0))?;
            if !already_deleted {
                tombstone_intent(tx, &mut ticket)?;
                stop_deleted_intent(tx, &ticket)?;
            }
            Ok(ticket)
        });
        if result.is_ok() { self.wake_scheduler(); }
        result
    }

    pub fn claim_publication_recovery(&self, runner: RunnerId, id: CommandId, grant: RequestId) -> Result<bool> {
        self.claim_result_recovery(runner, id, grant, false)
    }

    pub fn claim_review_snapshot_recovery(&self, runner: RunnerId, id: CommandId, grant: RequestId) -> Result<bool> {
        self.claim_result_recovery(runner, id, grant, true)
    }

    pub fn pending_review_snapshot_recoveries(&self, runner: RunnerId) -> Result<Vec<CommandId>> {
        self.pending_result_recoveries(runner, true)
    }

    pub fn pending_publication_recoveries(&self, runner: RunnerId) -> Result<Vec<CommandId>> {
        self.pending_result_recoveries(runner, false)
    }

    fn pending_result_recoveries(&self, runner: RunnerId, review_snapshot: bool) -> Result<Vec<CommandId>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let ids = {
            let mut query = tx.prepare(
                "SELECT c.id FROM tickets t
                 JOIN commands c ON c.id=json_extract(t.data,'$.execution_command')
                 WHERE c.runner_id=?1 AND c.state='succeeded'
                   AND json_extract(t.data,'$.status') IN ('in_progress','blocked')
                   AND COALESCE(json_extract(t.data,'$.draft'),0)=0
                   AND (?2=0 OR EXISTS(SELECT 1 FROM scheduler_runs r WHERE r.ticket_id=t.id AND r.agent_id=c.id))
                 GROUP BY c.id ORDER BY MIN(c.rowid)",
            )?;
            let rows = query.query_map(params![runner.to_string(), review_snapshot], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut pending = Vec::new();
        for id in ids {
            let id = id.parse().map_err(|_| StoreError::Internal)?;
            if recoverable_publication(&tx, runner, id, review_snapshot)?.is_some() {
                pending.push(id);
            }
        }
        Ok(pending)
    }

    fn claim_result_recovery(&self, runner: RunnerId, id: CommandId, grant: RequestId, review_snapshot: bool) -> Result<bool> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((_, owner)) = recoverable_publication(&tx, runner, id, review_snapshot)? else { return Ok(false); };
        let now = super::super::unix_ms();
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM runner_restart_holds WHERE runner_id=?1 AND expires_ms>?2)",
            params![runner.to_string(), now], |row| row.get::<_, bool>(0))? {
            return Ok(false);
        }
        tx.execute("DELETE FROM publication_recovery_holds WHERE expires_ms<=?1", [now])?;
        let acquired = tx.execute("INSERT OR IGNORE INTO publication_recovery_holds
            (project_id,command_id,runner_id,grant_id,expires_ms) VALUES(?1,?2,?3,?4,?5)",
            params![owner.project_id.to_string(), id.to_string(), runner.to_string(), grant.to_string(), now.saturating_add(120_000)])? == 1;
        tx.commit()?;
        Ok(acquired)
    }

    pub fn release_publication_recovery(&self, runner: RunnerId, id: CommandId, grant: RequestId) -> Result<()> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        db.execute("DELETE FROM publication_recovery_holds WHERE command_id=?1 AND runner_id=?2 AND grant_id=?3",
            params![id.to_string(), runner.to_string(), grant.to_string()])?;
        self.wake_scheduler();
        Ok(())
    }

    pub fn report_recovered_publication(&self, runner: RunnerId, id: CommandId, body: ReportRecoveredPublication) -> Result<()> {
        let oid = |value: &str| matches!(value.len(), 40 | 64)
            && value.bytes().all(|b| b.is_ascii_hexdigit());
        if !oid(&body.snapshot_commit) || !oid(&body.target_commit)
            || body.snapshot_commit.len() != body.target_commit.len()
        {
            return Err(StoreError::Invalid("Invalid Git commit IDs"));
        }
        self.mutate(body.request_id, &format!("publication.recover/{runner}/{id}"), &body, |tx| {
            let (mut command, mut owner) = recoverable_publication(tx, runner, id, false)?
                .ok_or(StoreError::WorkConflict)?;
            if command.ownership_epoch != body.ownership_epoch
                || command.workspace_spec.as_ref().is_none_or(|s| s.base_branch != body.base_branch)
                || tx.execute("DELETE FROM publication_recovery_holds
                      WHERE command_id=?1 AND runner_id=?2 AND grant_id=?3 AND expires_ms>?4",
                    params![id.to_string(), runner.to_string(), body.grant_id.to_string(), super::super::unix_ms()])? != 1
            {
                return Err(StoreError::WorkConflict);
            }
            let Some(CommandOutput::Agent { agent }) = &mut command.result else {
                return Err(StoreError::WorkConflict);
            };
            let previous = agent.summary.lines().filter(|line|
                !line.starts_with("Ergebnis im Arbeitsbereich erhalten; automatische Uebernahme in den Projektordner blockiert:")
            ).collect::<Vec<_>>().join("\n");
            agent.summary = format!("{PUBLICATION_CONFIRMED_PREFIX}\nGit: {} @ {} (Snapshot {})\n\nVorheriger Bericht:\n{}",
                body.base_branch, body.target_commit, body.snapshot_commit, previous);
            let mut end = agent.summary.len().min(32768);
            while !agent.summary.is_char_boundary(end) { end -= 1; }
            agent.summary.truncate(end);
            command.agent_progress = Some(agent.clone());
            super::work::save_command(tx, &command, "succeeded")?;
            set_publication_state(tx, &owner, id, TicketState::Done,
                "Factory hat das gesicherte Ergebnis auf dem Zielbranch bestätigt.")?;
            super::work::save_stream(tx, &mut owner, StreamState::Review)?;
            event(tx, "publication.recovered", id.to_string(), &body)?;
            Ok(())
        })
    }

    pub fn report_recovered_review_snapshot(&self, runner: RunnerId, id: CommandId, body: ReportRecoveredReviewSnapshot) -> Result<()> {
        if !matches!(body.snapshot_commit.len(), 40 | 64)
            || !body.snapshot_commit.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(StoreError::Invalid("Invalid Git snapshot ID"));
        }
        self.mutate(body.request_id, &format!("review-snapshot.recover/{runner}/{id}"), &body, |tx| {
            let (mut command, mut owner) = recoverable_publication(tx, runner, id, true)?
                .ok_or(StoreError::WorkConflict)?;
            if command.ownership_epoch != body.ownership_epoch
                || tx.execute("DELETE FROM publication_recovery_holds
                      WHERE command_id=?1 AND runner_id=?2 AND grant_id=?3 AND expires_ms>?4",
                    params![id.to_string(), runner.to_string(), body.grant_id.to_string(), super::super::unix_ms()])? != 1
            {
                return Err(StoreError::WorkConflict);
            }
            let Some(CommandOutput::Agent { agent }) = &mut command.result else {
                return Err(StoreError::WorkConflict);
            };
            let report = agent.summary.split_once("\n\nAgentenergebnis:\n")
                .map_or(agent.summary.as_str(), |(_, report)| report);
            let evidence = if body.unchanged_tree { format!("\n{NO_GIT_CHANGES_MARKER}") } else { String::new() };
            agent.summary = format!("{PUBLICATION_PREPARED_PREFIX}{evidence}\nReview-Snapshot {} nachträglich gesichert.\n\nAgentenergebnis:\n{report}", body.snapshot_commit);
            let mut end = agent.summary.len().min(32768);
            while !agent.summary.is_char_boundary(end) { end -= 1; }
            agent.summary.truncate(end);
            command.agent_progress = Some(agent.clone());
            command.agent_spec.as_mut().ok_or(StoreError::Internal)?.defer_publication = true;
            super::work::save_command(tx, &command, "succeeded")?;
            set_publication_state(tx, &owner, id, TicketState::Validating, "")?;
            super::work::save_stream(tx, &mut owner, StreamState::Review)?;
            event(tx, "validation.snapshot_recovered", id.to_string(), &body)?;
            Ok(())
        })?;
        self.wake_scheduler();
        Ok(())
    }

    pub fn reconcile_publication(&self, id: TicketId, body: ReconcilePublication) -> Result<Ticket> {
        let oid = |value: &str| matches!(value.len(), 40 | 64)
            && value.bytes().all(|b| b.is_ascii_hexdigit());
        if !oid(&body.target_commit) || !oid(&body.integrated_snapshot)
            || body.target_commit.len() != body.integrated_snapshot.len()
        {
            return Err(StoreError::Invalid("Invalid Git commit IDs"));
        }
        self.mutate(body.request_id, &format!("task.reconcile_publication/{id}"), &body, |tx| {
            let raw: String = tx.query_row(
                "SELECT t.data FROM tickets t JOIN active_projects p ON p.id=t.project_id
                 WHERE t.id=?1 AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)",
                [id.to_string()], |row| row.get(0),
            ).optional()?.ok_or(StoreError::NotFound)?;
            let mut ticket: Ticket = serde_json::from_str(&raw)?;
            super::draft_resets::require_not_resetting(tx, ticket.id)?;
            require_no_publication_recovery(tx, ticket.project_id)?;
            if ticket.revision != body.expected_revision {
                return Err(StoreError::RevisionConflict(ticket.revision));
            }
            if ticket.draft || ticket.status == TicketState::Cancelled
                || ticket.execution_command != Some(body.command_id)
            {
                return Err(StoreError::WorkConflict);
            }
            let raw: String = tx.query_row("SELECT data FROM commands WHERE id=?1",
                [body.command_id.to_string()], |row| row.get(0),
            ).optional()?.ok_or(StoreError::NotFound)?;
            let mut command: RunnerCommand = serde_json::from_str(&raw)?;
            let mut owner = super::work::stream(tx, command.stream_id)?;
            let raw: String = tx.query_row("SELECT data FROM active_projects WHERE id=?1",
                [ticket.project_id.to_string()], |row| row.get(0))?;
            let project: Project = serde_json::from_str(&raw)?;
            if owner.project_id != ticket.project_id || !owner.ticket_ids.contains(&id)
                || project.base_branch != body.base_branch
                || command.workspace_spec.as_ref().is_none_or(|w| w.base_branch != body.base_branch)
                || command.kind != CommandKind::RunAgent || command.state != CommandState::Succeeded
                || !command_publication_blocked(&command)
                || command.agent_spec.as_ref().is_none_or(|s| s.read_only)
            {
                return Err(StoreError::WorkConflict);
            }
            // Legacy successful runs already hold their original archive. Keep
            // the historical result in the record and prepend the later proof.
            let Some(CommandOutput::Agent { agent }) = &mut command.result else {
                return Err(StoreError::WorkConflict);
            };
            let old = std::mem::take(&mut agent.summary);
            let historical = old.lines().filter(|line|
                !line.starts_with("Ergebnis im Arbeitsbereich erhalten; automatische Uebernahme in den Projektordner blockiert:")
            ).collect::<Vec<_>>().join("\n");
            agent.summary = format!("{PUBLICATION_CONFIRMED_PREFIX}\nGit: {} @ {} (Snapshot {})\n\nAgentenergebnis:\n{}",
                body.base_branch, body.target_commit, body.integrated_snapshot, historical.trim());
            let mut end = agent.summary.len().min(32768);
            while !agent.summary.is_char_boundary(end) { end -= 1; }
            agent.summary.truncate(end);
            command.agent_progress = Some(agent.clone());
            super::work::save_command(tx, &command, "succeeded")?;
            ticket.status = TicketState::Done;
            ticket.routing_note = "Nachträgliche Übernahme auf dem Zielbranch bestätigt.".into();
            super::scheduler::save_ticket(tx, &mut ticket)?;
            let latest: bool = tx.query_row("SELECT id=?1 FROM commands WHERE stream_id=?2 ORDER BY rowid DESC LIMIT 1",
                params![command.id.to_string(), owner.id.to_string()], |row| row.get(0))?;
            if latest && owner.status == StreamState::Blocked {
                super::work::save_stream(tx, &mut owner, StreamState::Review)?;
            }
            event(tx, "publication.reconciled", id.to_string(), &body)?;
            Ok(ticket)
        })
    }

    pub fn delete_failed_task(&self, id: TicketId, body: DeleteFailedTask) -> Result<Ticket> {
        let result = self.mutate(body.request_id, &format!("task.delete_failed/{id}"), &body, |tx| {
            let raw: String = tx.query_row(
                "SELECT t.data FROM tickets t JOIN active_projects p ON p.id=t.project_id WHERE t.id=?1",
                [id.to_string()], |row| row.get(0),
            ).optional()?.ok_or(StoreError::NotFound)?;
            let mut ticket: Ticket = serde_json::from_str(&raw)?;
            if ticket.revision != body.expected_revision {
                return Err(StoreError::RevisionConflict(ticket.revision));
            }
            if ticket.draft || !matches!(ticket.status, TicketState::InProgress | TicketState::Blocked)
                || ticket.execution_command != Some(body.failed_command)
            {
                return Err(StoreError::WorkConflict);
            }
            let raw: String = tx.query_row(
                "SELECT c.data FROM commands c JOIN streams s ON s.id=c.stream_id WHERE c.id=?1 AND s.project_id=?2",
                params![body.failed_command.to_string(), ticket.project_id.to_string()], |row| row.get(0),
            ).optional()?.ok_or(StoreError::WorkConflict)?;
            let failed: RunnerCommand = serde_json::from_str(&raw)?;
            if failed.state != CommandState::Failed
                || !matches!(failed.kind, CommandKind::PrepareWorkspace | CommandKind::RunAgent | CommandKind::TargetGit)
            {
                return Err(StoreError::WorkConflict);
            }
            delete_intent_in(tx, &mut ticket)?;
            Ok(ticket)
        });
        if result.is_ok() { self.wake_scheduler(); }
        result
    }

    pub fn delete_inactive_task(&self, id: TicketId, body: DeleteInactiveTask) -> Result<Ticket> {
        let result = self.mutate(body.request_id, &format!("task.delete_inactive/{id}"), &body, |tx| {
            let raw: String = tx.query_row(
                "SELECT t.data FROM tickets t JOIN active_projects p ON p.id=t.project_id
                 WHERE t.id=?1 AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)",
                [id.to_string()], |row| row.get(0),
            ).optional()?.ok_or(StoreError::NotFound)?;
            let mut ticket: Ticket = serde_json::from_str(&raw)?;
            if ticket.revision != body.expected_revision {
                return Err(StoreError::RevisionConflict(ticket.revision));
            }
            if ticket.execution_command != body.execution_command {
                return Err(StoreError::WorkConflict);
            }
            if let Some(command) = ticket.execution_command {
                let state: Option<String> = tx.query_row(
                    "SELECT c.state FROM commands c JOIN streams s ON s.id=c.stream_id
                     WHERE c.id=?1 AND s.project_id=?2",
                    params![command.to_string(), ticket.project_id.to_string()], |row| row.get(0),
                ).optional()?;
                if !matches!(state.as_deref(), Some("succeeded" | "failed")) {
                    return Err(StoreError::WorkConflict);
                }
            } else if tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM commands c JOIN stream_tickets st ON st.stream_id=c.stream_id
                 WHERE st.ticket_id=?1 AND c.state IN ('pending','accepted'))",
                [id.to_string()], |row| row.get::<_, bool>(0),
            )? {
                return Err(StoreError::WorkConflict);
            }
            delete_intent_in(tx, &mut ticket)?;
            Ok(ticket)
        });
        if result.is_ok() { self.wake_scheduler(); }
        result
    }

    pub fn retry_task(&self, id: TicketId, body: RetryTask) -> Result<Ticket> {
        let result = self.mutate(body.request_id, &format!("task.retry/{id}"), &body,
            |tx| retry_failed(tx, id, &body));
        if result.is_ok() { self.wake_scheduler(); }
        result
    }
    pub fn begin_task_edit(&self, id: TicketId, body: BeginTaskEdit) -> Result<TaskEdit> {
        self.mutate(body.request_id,&format!("task.edit/{id}"),&body,|tx| {
            let raw:Option<String>=tx.query_row("SELECT data FROM tickets WHERE id=?1",[id.to_string()],|r|r.get(0)).optional()?;
            let mut ticket:Ticket=serde_json::from_str(&raw.ok_or(StoreError::NotFound)?)?;
            if ticket.revision!=body.expected_revision {return Err(StoreError::RevisionConflict(ticket.revision));}
            if !editable(tx,&ticket)? {return Err(StoreError::Invalid("Die Factory hat diese Aufgabe bereits übernommen. Bearbeiten ist nicht mehr möglich"));}
            let held:Option<String>=tx.query_row("SELECT token FROM task_edits WHERE ticket_id=?1",[id.to_string()],|r|r.get(0)).optional()?;
            let token=if let Some(held)=held {
                if body.edit_token.map(|id|id.to_string()).as_deref()!=Some(&held) {return Err(StoreError::Invalid("Diese Aufgabe wird bereits in einem anderen Entwurf bearbeitet"));}
                body.edit_token.ok_or(StoreError::Internal)?
            } else {
                let token=body.edit_token.unwrap_or(body.request_id);
                tx.execute("INSERT INTO task_edits(ticket_id,token) VALUES(?1,?2)",params![id.to_string(),token.to_string()])?;
                ticket.draft=true;
                ticket.revision.0=ticket.revision.0.checked_add(1).ok_or(StoreError::Internal)?;
                ticket.updated_unix_ms=super::super::unix_ms();
                ticket.routing_note="In Bearbeitung als Entwurf — nicht zur Ausführung freigegeben".into();
                tx.execute("UPDATE tickets SET revision=?1,data=?2 WHERE id=?3",params![ticket.revision.0,serde_json::to_string(&ticket)?,id.to_string()])?;
                event(tx,"task.edit_started",id.to_string(),&ticket)?;
                token
            };
            let local_mcps = super::mcp::task_optional(tx, ticket.id)?;
            Ok(TaskEdit {ticket,edit_token:token, local_mcps})
        })
    }
    pub fn delete_project(&self, id: ProjectId, body: DeleteProject) -> Result<Project> {
        self.mutate(body.request_id,&format!("project.delete/{id}"),&body,|tx| {
            let raw:Option<String>=tx.query_row("SELECT data FROM active_projects WHERE id=?1",[id.to_string()],|row|row.get(0)).optional()?;
            let mut project:Project=serde_json::from_str(&raw.ok_or(StoreError::NotFound)?)?;
            require_no_publication_recovery(tx, id)?;
            if project.revision!=body.expected_revision {return Err(StoreError::RevisionConflict(project.revision));}
            if body.confirm_name!=project.name {return Err(StoreError::Invalid("Zum Löschen den vollständigen Projektnamen bestätigen"));}
            let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands c JOIN streams s ON s.id=c.stream_id WHERE s.project_id=?1 AND c.state IN ('pending','accepted'))",[id.to_string()],|row|row.get(0))?;
            let resetting:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM draft_resets d JOIN streams s ON s.id=d.stream_id WHERE s.project_id=?1 AND d.completed=0)",[id.to_string()],|row|row.get(0))?;
            if busy || resetting {return Err(StoreError::Invalid("Projekt hat noch laufende oder eingereihte Aufträge und kann nicht gelöscht werden"));}
            project.deleted=true;
            project.paused=true;
            project.revision.0=project.revision.0.checked_add(1).ok_or(StoreError::Internal)?;
            tx.execute("UPDATE projects SET data=?1 WHERE id=?2",params![serde_json::to_string(&project)?,id.to_string()])?;
            for table in ["scheduler_policies","supervisor_policies"] {
                tx.execute(&format!("UPDATE {table} SET data=json_set(data,'$.enabled',json('false'),'$.revision',json_extract(data,'$.revision')+1) WHERE project_id=?1"),[id.to_string()])?;
            }
            event(tx,"project.deleted",id.to_string(),&project)?;
            Ok(project)
        })
    }
    pub fn set_session_limit(&self, id: ProjectId, b: SetSessionLimit) -> Result<Project> {
        self.mutate(
            b.request_id,
            &format!("project.session_limit/{id}"),
            &b,
            |tx| {
                if !(1..=16).contains(&b.max_sessions) {
                    return Err(StoreError::Invalid(
                        "Sessionlimit muss zwischen 1 und 16 liegen",
                    ));
                }
                let data: Option<String> = tx
                    .query_row(
                        "SELECT data FROM active_projects WHERE id=?1",
                        [id.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?;
                let mut p: Project = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if p.revision != b.expected_revision {
                    return Err(StoreError::RevisionConflict(p.revision));
                }
                p.max_sessions = b.max_sessions;
                p.revision.0 += 1;
                tx.execute(
                    "UPDATE projects SET data=?1 WHERE id=?2",
                    params![serde_json::to_string(&p)?, id.to_string()],
                )?;
                super::scheduler::set_limit(tx, id, b.max_sessions)?;
                event(tx, "project.session_limit", id.to_string(), &p)?;
                Ok(p)
            },
        )
    }
    pub fn pause_project(&self, id: ProjectId, b: PauseProject) -> Result<Project> {
        self.mutate(b.request_id, &format!("project.pause/{id}"), &b, |tx| {
            require_no_publication_recovery(tx, id)?;
            let data: Option<String> = tx
                .query_row(
                    "SELECT data FROM active_projects WHERE id=?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            let mut p: Project = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
            if p.revision != b.expected_revision {
                return Err(StoreError::RevisionConflict(p.revision));
            }
            p.paused = b.paused;
            p.revision.0 += 1;
            tx.execute(
                "UPDATE projects SET data=?1 WHERE id=?2",
                params![serde_json::to_string(&p)?, id.to_string()],
            )?;
            event(tx, "project.paused", id.to_string(), &p)?;
            Ok(p)
        })
    }
    pub fn save_task(&self, b: SaveTask) -> Result<Ticket> {
        self.submit_task_in(
            SubmitTask {
                edit_token: None,
                task: b,
                draft: false,
                files: vec![],
            },
            true,
            None,
            None,
            false,
        )
    }
    pub fn submit_task(&self, submission: SubmitTask) -> Result<Ticket> {
        self.submit_task_in(submission, false, None, None, false)
    }
    pub fn submit_task_with_model(&self, body: SubmitTaskWithModel) -> Result<Ticket> {
        self.submit_task_in(body.submission, false, Some(&body.implementer), body.local_mcps.as_deref(), false)
    }
    pub fn prompt_session(&self, root: CommandId, body: PromptSession) -> Result<Ticket> {
        let project = {
            let db = self.0.lock().map_err(|_| StoreError::Internal)?;
            db.query_row("SELECT s.project_id FROM commands c JOIN streams s ON s.id=c.stream_id WHERE c.id=?1",
                [root.to_string()], |r| r.get::<_, String>(0)).optional()?.ok_or(StoreError::NotFound)?
                .parse().map_err(|_| StoreError::Internal)?
        };
        self.submit_task_in(SubmitTask {
            task: SaveTask {
                request_id: body.request_id, id: None, expected_revision: Revision(0),
                project_id: project, requested_session: Some(root), prompt: body.prompt, images: body.images,
            },
            files: body.files, draft: false, edit_token: None,
        }, false, None, None, true)
    }
    fn submit_task_in(&self, submission: SubmitTask, legacy: bool, implementer: Option<&Option<ModelSelection>>, requested_mcps: Option<&[String]>, direct: bool) -> Result<Ticket> {
        let b = &submission.task;
        // Preserve the legacy fingerprint so a previously accepted SaveTask is replayable.
        #[derive(serde::Serialize)]
        #[serde(untagged)]
        enum Fingerprint<'a> {
            Legacy(&'a SaveTask),
            Extended(&'a SubmitTask),
            SessionPrompt(&'a SubmitTask),
            WithModel { submission: &'a SubmitTask, implementer: &'a Option<ModelSelection>,
                #[serde(skip_serializing_if = "Option::is_none")] local_mcps: Option<&'a [String]> },
        }
        let fingerprint = if direct {
            Fingerprint::SessionPrompt(&submission)
        } else if legacy {
            Fingerprint::Legacy(b)
        } else if let Some(implementer) = implementer {
            Fingerprint::WithModel { submission: &submission, implementer, local_mcps: requested_mcps }
        } else {
            Fingerprint::Extended(&submission)
        };
         let result = self.mutate(b.request_id,if direct { "session.prompt" } else if legacy { "task.save" } else if implementer.is_some() { "task.submit_with_model" } else { "task.submit" },&fingerprint,|tx|{
            if b.prompt.trim().is_empty() && b.images.is_empty() && submission.files.is_empty() {
                return Err(StoreError::Invalid("Intent braucht Text oder Anhänge"));
            }
            if b.prompt.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
                return Err(StoreError::Invalid("Prompt enthält ungültige Steuerzeichen"));
            }
            validate_images(&b.images).map_err(StoreError::Invalid)?;
            validate_files(&submission.files).map_err(StoreError::Invalid)?;
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM active_projects WHERE id=?1)",[b.project_id.to_string()],|r|r.get(0))?;if !exists{return Err(StoreError::NotFound);}
            let now=super::super::unix_ms();
             let session = if let Some(root)=b.requested_session {
                let data:Option<String>=tx.query_row("SELECT c.data FROM commands c JOIN streams s ON s.id=c.stream_id WHERE c.id=?1 AND s.project_id=?2",params![root.to_string(),b.project_id.to_string()],|r|r.get(0)).optional()?;
                 let c:RunnerCommand=serde_json::from_str(&data.ok_or(StoreError::Invalid("Session gehoert nicht zu diesem Projekt"))?)?;
                 if super::draft_resets::retired(tx, c.stream_id)? { return Err(StoreError::WorkConflict); }
                 if c.kind!=CommandKind::RunAgent || c.session_root()!=root || c.agent_spec.as_ref().is_none_or(|a|a.read_only || a.role == factory_protocol::directs::ROLE){return Err(StoreError::Invalid("Keine auswaehlbare Arbeitssession"));}
                 if direct && tx.query_row(
                     "SELECT EXISTS(SELECT 1 FROM runtime_terminations WHERE stream_id=?1)",
                     [c.stream_id.to_string()], |r| r.get::<_, bool>(0))? {
                     return Err(StoreError::Invalid("Session-Laufzeit wurde beendet oder wird beendet"));
                 }
                 c.agent_spec
             } else { None };
            let mut t=if let Some(id)=b.id {
                let data:Option<String>=tx.query_row("SELECT data FROM tickets WHERE id=?1",[id.to_string()],|r|r.get(0)).optional()?;let t:Ticket=serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if t.project_id!=b.project_id{return Err(StoreError::WorkConflict);}
                if legacy && !t.files.is_empty() { return Err(StoreError::Invalid("Dieser Client kann Datei-Anhänge nicht bearbeiten; Desktop-Aufgabendialog verwenden")); }
                if t.revision!=b.expected_revision{return Err(StoreError::RevisionConflict(t.revision));}
                if !editable(tx,&t)? {return Err(StoreError::Invalid("Aufgabe bereits verarbeitet; neuen Prompt als Folgeaufgabe speichern"));}
                let held:Option<String>=tx.query_row("SELECT token FROM task_edits WHERE ticket_id=?1",[id.to_string()],|r|r.get(0)).optional()?;
                if held.is_some() && held!=submission.edit_token.map(|id|id.to_string()) {return Err(StoreError::Invalid("Bearbeitung gehört zu einem anderen Entwurf"));}
                t
            }else{if b.expected_revision!=Revision(0){return Err(StoreError::RevisionConflict(Revision(0)));}Ticket{key:super::numbering::next_reference(tx,b.project_id)?,draft:false,files:vec![],execution_command:None,requested_session:None,session_prompt_request:None,routing_note:String::new(),id:TicketId::new(),project_id:b.project_id,revision:Revision(0),title:String::new(),description:String::new(),acceptance_criteria:String::new(),priority:1,status:TicketState::Backlog,created_unix_ms:now,updated_unix_ms:now,depends_on:vec![],prompt:None,images:vec![]}};
            t.draft = submission.draft;
            t.session_prompt_request = direct.then_some(b.request_id);
            t.files = submission.files.clone();
            t.title=b.prompt.lines().next().unwrap_or("").chars().take(80).collect();if t.title.trim().is_empty(){t.title=if !submission.files.is_empty() { "Dateiaufgabe" } else { "Bildaufgabe" }.into();}
            t.description=b.prompt.clone();t.prompt=Some(b.prompt.clone());t.images=b.images.clone();t.revision.0+=1;t.updated_unix_ms=now;
             t.requested_session=b.requested_session;t.routing_note=if t.draft { "Entwurf — noch nicht freigegeben".into() } else if b.requested_session.is_some(){"Fest an ausgewaehlte Session gebunden".into()}else{"Wartet auf Orchestrator".into()};
               let models = if direct {
                   let agent = session.as_ref().ok_or(StoreError::Invalid("Arbeitssession fehlt"))?;
                   if super::models::factory(tx)?.is_some() {
                       let model = ModelSelection { access_id: agent.profile.provider_id.clone(), model_id: agent.profile.model_id.clone() };
                       super::models::require_available(tx, &model)?;
                       Some(factory_protocol::models::TaskModels {
                           implementer_override: Some(model.clone()), implementer: Some(model),
                           orchestrator: None,
                           supervisor: super::models::role_selection(tx, t.project_id, "supervisor")?,
                            memorizer: super::models::role_selection(tx, t.project_id, "memorizer")?,
                            reviewer: super::models::role_selection(tx, t.project_id, "reviewer")?,
                       })
                   } else { None }
               } else { super::models::for_task(tx, t.id, t.project_id, implementer, t.draft)? };
               let selected_skills = super::skills::selected(tx, t.project_id)?;
               let repo_skills = if direct {
                   selected_skills.for_session_prompt(session.as_ref().ok_or(StoreError::Internal)?)
                       .ok_or(StoreError::Invalid("Mindestens ein Skill dieser Session ist nicht mehr freigegeben; neue Session wählen"))?
               } else { selected_skills };
                let optional_mcps = match requested_mcps {
                    Some(requested) => requested.to_vec(),
                    None if b.id.is_none() && !direct => super::mcp::default_optional(tx, t.project_id)?,
                    None => super::mcp::task_optional(tx, t.id)?,
               };
                let local_mcps = if direct {
                    let agent = session.as_ref().ok_or(StoreError::Invalid("Arbeitssession fehlt"))?;
                    let pinned = McpProfile { local: agent.local_mcps.clone(), remote: agent.remote_mcps.clone() };
                    if !super::mcp::allowed(tx, t.project_id, &pinned)? {
                        return Err(StoreError::Invalid("MCP-Freigabe der Session ist nicht mehr gültig"));
                    }
                    pinned
                } else { super::mcp::resolve(tx, t.project_id, &optional_mcps)? };
                if !t.draft && session.as_ref().is_some_and(|agent|
                    (!direct && !repo_skills.matches(agent)) || !local_mcps.matches(agent)) {
                  return Err(StoreError::Invalid("Skill-Freigabe oder MCP-Auswahl passt nicht zur ausgewählten Session; neue Session wählen"));
              }
             if let Some((model, agent)) = models.as_ref().and_then(|m| m.implementer.as_ref()).zip(session.as_ref()) {
                 if agent.profile.provider_id != model.access_id || agent.profile.model_id != model.model_id {
                     return Err(StoreError::Invalid("Gewähltes Modell oder Zugang passt nicht zur ausgewählten Session; neue Session wählen"));
                 }
             }
              tx.execute("INSERT INTO tickets(id,project_id,revision,data) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data",params![t.id.to_string(),t.project_id.to_string(),t.revision.0,serde_json::to_string(&t)?])?;
              if !t.draft {
                  super::skills::pin(tx, t.id, &repo_skills)?;
               }
               super::mcp::pin(tx, t.id, local_mcps, optional_mcps)?;
             if let Some(models) = models {
                 super::models::save_ticket(tx, t.id, &models)?;
             }
            tx.execute("DELETE FROM task_edits WHERE ticket_id=?1",[t.id.to_string()])?;
            if !t.draft && legacy { super::scheduler::activate_prompts(tx,t.project_id)?; }
             if direct {
                 super::sessions::reopen_for_prompt(tx, b.requested_session.ok_or(StoreError::Internal)?, t.id)?;
             }
             event(tx,"task.saved",t.id.to_string(),&t)?;Ok(t)
        });
        if result.is_ok() && !submission.draft {
            self.wake_scheduler();
        }
        result
    }
}

pub(super) fn recover_stopped(tx: &Transaction<'_>, project: ProjectId) -> Result<()> {
    let candidates = {
        let mut query = tx.prepare(
            "SELECT t.data FROM tickets t JOIN commands c ON c.id=json_extract(t.data,'$.execution_command')
             JOIN archive_status a ON a.command_id=c.id
             WHERE t.project_id=?1 AND json_extract(t.data,'$.status') IN ('in_progress','blocked')
               AND c.state='failed' AND json_extract(c.data,'$.kind')='run_agent'
               AND instr(json_extract(c.data,'$.result.agent.summary'),?2)=1
               AND json_extract(a.data,'$.final_scan_complete')=1
               AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)
               AND NOT EXISTS(SELECT 1 FROM events e WHERE e.aggregate_id=t.id AND e.kind='task.session_recovered')
               AND NOT EXISTS(SELECT 1 FROM draft_resets d WHERE d.stream_id=c.stream_id)
             ORDER BY c.rowid",
        )?;
        let rows = query.query_map(params![project.to_string(), SESSION_STOPPED_PREFIX], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for raw in candidates {
        let ticket: Ticket = serde_json::from_str(&raw)?;
        let Some(previous) = ticket.execution_command else { continue; };
        tx.execute_batch("SAVEPOINT resume_stopped_intent")?;
        match retry_failed_inner(tx, ticket.id, &RetryTask {
            request_id: RequestId::new(), expected_revision: ticket.revision, previous_command: previous,
        }, true) {
            Ok(_) => {}
            Err(StoreError::Invalid(_) | StoreError::WorkConflict | StoreError::RevisionConflict(_)) =>
                tx.execute_batch("ROLLBACK TO resume_stopped_intent")?,
            Err(error) => return Err(error),
        }
        tx.execute_batch("RELEASE resume_stopped_intent")?;
    }
    Ok(())
}

pub(super) fn retry_failed(tx: &Transaction<'_>, id: TicketId, body: &RetryTask) -> Result<Ticket> {
    let raw: String = tx.query_row("SELECT data FROM tickets WHERE id=?1",
        [id.to_string()], |r| r.get(0)).optional()?.ok_or(StoreError::NotFound)?;
    let ticket: Ticket = serde_json::from_str(&raw)?;
    if ticket.revision == body.expected_revision && ticket.execution_command == Some(body.previous_command) {
        super::routing::recover_unusable_workspaces(tx, ticket.project_id, Some(id))?;
        let raw: String = tx.query_row("SELECT data FROM tickets WHERE id=?1",
            [id.to_string()], |r| r.get(0))?;
        let updated: Ticket = serde_json::from_str(&raw)?;
        if updated.execution_command != ticket.execution_command {
            return Ok(updated);
        }
    }
    retry_failed_inner(tx, id, body, false)
}

pub(super) fn continue_failed_agent(tx: &Transaction<'_>, ticket: &mut Ticket, failed: &RunnerCommand) -> Result<()> {
    let owner = super::work::stream(tx, failed.stream_id)?;
    if ticket.execution_command != Some(failed.id)
        || failed.kind != CommandKind::RunAgent || failed.state != CommandState::Failed
        || owner.project_id != ticket.project_id || !owner.ticket_ids.contains(&ticket.id)
        || failed.requires_target_git()
        || failed.agent_spec.as_ref().is_none_or(|s| s.read_only || !s.role.is_empty())
        || !matches!(&failed.result, Some(CommandOutput::Agent { agent }) if agent.phase == AgentPhase::Failed)
        || !tx.query_row("SELECT id=?1 FROM commands WHERE stream_id=?2 ORDER BY rowid DESC LIMIT 1",
            params![failed.id.to_string(), owner.id.to_string()], |r| r.get::<_, bool>(0))?
        || tx.query_row("SELECT EXISTS(SELECT 1 FROM questions WHERE command_id=?1 AND json_extract(data,'$.state')<>'answered')",
            [failed.id.to_string()], |r| r.get::<_, bool>(0))?
    {
        return Err(StoreError::WorkConflict);
    }
    let pins = super::models::ticket(tx, ticket.id)?;
    let skills = super::skills::for_ticket(tx, ticket.id, ticket.project_id)?;
    let mcps = super::mcp::for_ticket(tx, ticket.id, ticket.project_id)?;
    let mut next = Store::enqueue_intent_recovery(tx, owner.id, &EnqueueCommand {
        request_id: RequestId::new(), expected_revision: owner.revision, kind: CommandKind::RunAgent,
        prompt: Some(INTENT_RECOVERY_PROMPT.into()),
    }, failed.id, pins.as_ref().and_then(|m| m.implementer.as_ref()), (&skills, &mcps))?;
    if next.workspace_spec != failed.workspace_spec {
        return Err(StoreError::WorkConflict);
    }
    super::validation::defer_publication(tx, &mut next)?;
    tx.execute("UPDATE scheduler_runs SET agent_id=?1 WHERE ticket_id=?2 AND agent_id=?3",
        params![next.id.to_string(), ticket.id.to_string(), failed.id.to_string()])?;
    ticket.execution_command = Some(next.id);
    ticket.status = TicketState::InProgress;
    super::routing::note(tx, ticket, "Orchestrator setzt den fehlgeschlagenen Intent in derselben Session fort.")?;
    event(tx, "task.session_recovered", ticket.id.to_string(), &serde_json::json!({
        "ticket":ticket,"previous_command":failed.id,"command":next.id
    }))
}

pub(super) fn retry_failed_inner(tx: &Transaction<'_>, id: TicketId, body: &RetryTask, automatic: bool) -> Result<Ticket> {
    super::draft_resets::require_not_resetting(tx, id)?;
    let raw: Option<String> = tx.query_row("SELECT data FROM tickets WHERE id=?1", [id.to_string()], |r| r.get(0)).optional()?;
    let mut ticket: Ticket = serde_json::from_str(&raw.ok_or(StoreError::NotFound)?)?;
    if ticket.revision != body.expected_revision {
        return Err(StoreError::RevisionConflict(ticket.revision));
    }
    if ticket.draft || !matches!(ticket.status, TicketState::InProgress | TicketState::Blocked)
        || ticket.execution_command != Some(body.previous_command) {
        return Err(StoreError::Invalid("Intent hat keinen erneut einreihbaren fehlgeschlagenen Auftrag"));
    }
    let raw: String = tx.query_row("SELECT data FROM commands WHERE id=?1", [body.previous_command.to_string()], |r| r.get(0))?;
    let failed: RunnerCommand = serde_json::from_str(&raw)?;
    if !automatic && super::validation::retry(tx, &mut ticket, &failed)? {
        return Ok(ticket);
    }
    let owner = super::work::stream(tx, failed.stream_id)?;
    let latest: String = tx.query_row("SELECT id FROM commands WHERE stream_id=?1 ORDER BY rowid DESC LIMIT 1",
        [owner.id.to_string()], |r| r.get(0))?;
    if failed.state != CommandState::Failed || latest != failed.id.to_string()
        || failed.agent_spec.as_ref().is_some_and(|s| s.role == MERGER_ROLE)
        || failed.runner_id != owner.runner_id || failed.ownership_epoch != owner.ownership_epoch
        || owner.project_id != ticket.project_id
        || !matches!(failed.kind, CommandKind::PrepareWorkspace | CommandKind::TargetGit | CommandKind::RunAgent)
    {
        return Err(StoreError::WorkConflict);
    }
    if tx.query_row("SELECT EXISTS(SELECT 1 FROM questions WHERE command_id=?1 AND json_extract(data,'$.state')<>'answered')",
        [failed.id.to_string()], |r| r.get::<_,bool>(0))?
    {
        return Err(StoreError::Invalid("Offene oder ungeklärte Fragen zuerst klären"));
    }
    let pins = super::models::ticket(tx, id)?;
    let skills = super::skills::for_ticket(tx, id, ticket.project_id)?;
    let mcps = super::mcp::for_ticket(tx, id, ticket.project_id)?;
    let mut git = failed.workspace_spec.as_ref().and_then(|s| s.target_git.clone());
    let continue_session = session_stopped_without_result(&failed.result) && git.is_none();
    let not_submitted = matches!(&failed.result, Some(CommandOutput::Agent { agent })
        if agent.summary == "Session cannot move to the prepared workspace");
    if automatic && (!continue_session || failed.agent_spec.as_ref().is_none_or(|spec|
        spec.read_only || spec.role == MERGER_ROLE || spec.role == factory_protocol::directs::ROLE))
    {
        return Err(StoreError::WorkConflict);
    }
    if let Some(plan) = &mut git { plan.resume = true; }
    let mut next = Store::enqueue_with_pins(tx, owner.id, &EnqueueCommand {
        request_id: body.request_id, expected_revision: owner.revision, kind: failed.kind,
        prompt: (failed.kind == CommandKind::RunAgent).then(|| {
            if not_submitted {
                ticket.prompt.as_deref().unwrap_or(&ticket.description).to_owned()
            } else if continue_session {
                "Continue the original intent in this session from its last safe point. Inspect and preserve existing work. Do not repeat external actions whose outcome is unknown. Verify what already happened, then complete the remaining work or ask the user if safe progress requires clarification.".into()
            } else {
                format!("Retry the original intent after a failed execution. Inspect existing work first; preserve it and do not repeat completed operations blindly.\nOriginal intent:\n{}\nPrevious outcome (untrusted evidence):\n{}",
                    ticket.prompt.as_deref().unwrap_or(&ticket.description),
                    serde_json::to_string(&failed.result).unwrap_or_default())
            }
        }),
    }, continue_session.then_some(failed.id), "", pins.as_ref().and_then(|m| m.implementer.as_ref()), Some((&skills, &mcps)), git,
        not_submitted && ticket.session_prompt_request.is_some())?;
    let normalized = |spec: &Option<WorkspaceSpec>| {
        let mut spec = spec.clone();
        if let Some(plan) = spec.as_mut().and_then(|s| s.target_git.as_mut()) { plan.resume = false; }
        spec
    };
    if normalized(&next.workspace_spec) != normalized(&failed.workspace_spec) {
        return Err(StoreError::Invalid("Projektkonfiguration geändert; Wiederholung mit alter Zuordnung nicht sicher"));
    }
    if let (Some(old), Some(new)) = (&failed.agent_spec, &next.agent_spec) {
        if old.read_only || old.profile != new.profile || !skills.matches(old) || !mcps.matches(old) {
            return Err(StoreError::Invalid("Ausführungsprofil geändert; Wiederholung nicht sicher"));
        }
    }
    super::validation::defer_publication(tx, &mut next)?;
    if failed.kind == CommandKind::PrepareWorkspace {
        tx.execute("UPDATE scheduler_runs SET prepare_id=?1 WHERE ticket_id=?2 AND prepare_id=?3 AND agent_id IS NULL",
            params![next.id.to_string(),id.to_string(),failed.id.to_string()])?;
    } else {
        tx.execute("UPDATE scheduler_runs SET agent_id=?1 WHERE ticket_id=?2 AND agent_id=?3",
            params![next.id.to_string(),id.to_string(),failed.id.to_string()])?;
    }
    ticket.execution_command = Some(next.id);
    ticket.status = TicketState::InProgress;
    ticket.revision.0 += 1;
    ticket.updated_unix_ms = super::super::unix_ms();
    ticket.routing_note = if automatic {
        "Factory setzt die gestoppte Session für diesen Intent fort."
    } else {
        "Intent erneut eingereiht; vorhandene Ergebnisse werden vor Wiederholung geprüft"
    }.into();
    tx.execute("UPDATE tickets SET revision=?1,data=?2 WHERE id=?3",
        params![ticket.revision.0,serde_json::to_string(&ticket)?,id.to_string()])?;
    event(tx, if automatic { "task.session_recovered" } else { "task.retried" }, id.to_string(), &serde_json::json!({
        "ticket":ticket,"previous_command":failed.id,"command":next.id
    }))?;
    Ok(ticket)
}
