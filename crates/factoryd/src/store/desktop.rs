use super::*;

pub(super) fn require_intent_attention(tx: &Transaction<'_>, ticket: &Ticket, reason: &str) -> Result<()> {
    report_intent_problem(tx, ticket, reason, None)
}

pub(super) fn report_intent_problem(
    tx: &Transaction<'_>, ticket: &Ticket, reason: &str, solution: Option<IntentSolution>,
) -> Result<()> {
    let Some(command_id) = ticket.execution_command.or_else(||
        (solution == Some(IntentSolution::ReassignSession)).then_some(ticket.requested_session).flatten())
    else { return Ok(()); };
    let existing: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM events WHERE kind=?1 AND aggregate_id=?2 AND json_extract(payload,'$.command_id')=?3)",
        params![INTENT_ATTENTION_EVENT, ticket.id.to_string(), command_id.to_string()], |row| row.get(0),
    )?;
    if existing { return Ok(()); }
    let problem = IntentProblem { ticket_id: ticket.id, command_id, reason: reason.into(), solution };
    event(tx, INTENT_ATTENTION_EVENT, ticket.id.to_string(), &problem)
}

pub(super) fn require_execution_attention(tx: &Transaction<'_>, owner: &Workstream, command: CommandId, reason: &str) -> Result<()> {
    for id in &owner.ticket_ids {
        let raw: Option<String> = tx.query_row(
            "SELECT data FROM tickets t WHERE id=?1 AND json_extract(data,'$.execution_command')=?2
                AND json_extract(data,'$.status')='blocked'
                AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)",
            params![id.to_string(), command.to_string()], |row| row.get(0),
        ).optional()?;
        if let Some(raw) = raw {
            require_intent_attention(tx, &serde_json::from_str::<Ticket>(&raw)?, reason)?;
        }
    }
    Ok(())
}

pub(super) fn intent_problems(tx: &Transaction<'_>) -> Result<Vec<IntentProblem>> {
    let mut query = tx.prepare(
        "SELECT e.payload FROM events e JOIN tickets t ON t.id=e.aggregate_id
         JOIN active_projects p ON p.id=t.project_id
         WHERE e.kind=?1
           AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)
         ORDER BY e.sequence DESC",
    )?;
    let rows = query.query_map([INTENT_ATTENTION_EVENT], |row| row.get::<_, String>(0))?;
    rows.map(|raw| Ok(serde_json::from_str(&raw?)?)).collect()
}

/// Preserve names/types and all nonbinary fields; raw bytes remain in the original event.
fn defer_binary(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("mime").is_some()
                && map.get("data").is_some_and(serde_json::Value::is_string)
            {
                map.insert(
                    "data".into(),
                    serde_json::Value::String(
                        "[Anhangbytes: vollständiges Originalereignis laden]".into(),
                    ),
                );
            }
            for child in map.values_mut() {
                defer_binary(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                defer_binary(child);
            }
        }
        _ => {}
    }
}

pub(super) fn record_notification(
    tx: &Transaction<'_>,
    sequence: u64,
    kind: &str,
    id: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    let mut command_id = None;
    let mut question_id = None;
    let mut ticket_id: Option<TicketId> = None;
    let title;
    let dedup;
    if matches!(kind, "question.created" | "question.updated") {
        let q: Question = serde_json::from_value(payload.clone())?;
        let direct: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands c JOIN directs d ON d.stream_id=c.stream_id WHERE c.id=?1)",
            [q.command_id.to_string()], |row| row.get(0),
        )?;
        if direct { return Ok(()); }
        if !matches!(q.state, QuestionState::Pending | QuestionState::Rejected)
            || (q.state == QuestionState::Pending && q.orchestration_note == QUESTION_AUTOMATION_PENDING)
        {
            return Ok(());
        }
        command_id = Some(q.command_id);
        question_id = Some(q.id);
        title = "Antwort erforderlich";
        dedup = format!("question/{}", q.id);
    } else if kind == INTENT_ATTENTION_EVENT {
        let problem: IntentProblem = serde_json::from_value(payload.clone())?;
        command_id = Some(problem.command_id);
        ticket_id = Some(problem.ticket_id);
        title = "Intent braucht Aufmerksamkeit";
        dedup = format!("intent-attention/{}/{}", problem.ticket_id, problem.command_id);
    } else if kind == "scheduler.blocked" {
        ticket_id = id.parse().ok();
        if payload["missing_model"] == true {
            title = "Orchestrator wartet: Projektmodell fehlt";
            dedup = format!("missing-model/{id}");
        } else {
            title = "Aufgabe blockiert";
            dedup = format!("blocked/{id}");
        }
    } else {
        return Ok(());
    }
    let project: Option<String> = if let Some(command) = command_id {
        tx.query_row(
            "SELECT s.project_id FROM commands c JOIN streams s ON s.id=c.stream_id WHERE c.id=?1",
            [command.to_string()],
            |r| r.get(0),
        )
        .optional()?
    } else if let Some(ticket) = ticket_id {
        tx.query_row(
            "SELECT project_id FROM tickets WHERE id=?1",
            [ticket.to_string()],
            |r| r.get(0),
        )
        .optional()?
    } else {
        None
    };
    let n = Notification {
        id: sequence,
        project_id: project.and_then(|s| s.parse().ok()),
        command_id,
        question_id,
        ticket_id,
        title: title.into(),
        kind: kind.into(),
        created_unix_ms: super::super::unix_ms(),
        read: false,
    };
    tx.execute(
        "INSERT OR IGNORE INTO notifications(id,dedup,data) VALUES(?1,?2,?3)",
        params![sequence, dedup, serde_json::to_string(&n)?],
    )?;
    Ok(())
}

impl Store {
    /// One current version per message, ordered by first appearance. Revision
    /// churn from a single streaming tool must not evict the conversation.
    pub fn conversation_window(
        &self,
        id: CommandId,
        after: Option<u64>,
        before: Option<u64>,
    ) -> Result<ArchiveWindow> {
        if after.is_some() {
            return self.archive_window(id, after, before);
        }
        if before.is_some_and(|v| v > i64::MAX as u64) {
            return Err(StoreError::Invalid("Invalid conversation cursor"));
        }
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
        let status: Option<String> = tx
            .query_row(
                "SELECT data FROM archive_status WHERE command_id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let latest_sequence = tx.query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM archive_messages WHERE command_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        let mut stmt = tx.prepare("SELECT a.sequence,a.sample_revision,a.captured_unix_ms,a.data,m.position FROM archive_messages a JOIN (SELECT message_id,MIN(sequence) position,MAX(sequence) latest FROM archive_messages WHERE command_id=?1 GROUP BY message_id) m ON a.sequence=m.latest WHERE m.position<?2 ORDER BY m.position DESC LIMIT 33")?;
        let rows = stmt.query_map(
            params![id.to_string(), before.unwrap_or(i64::MAX as u64)],
            |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, u64>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, u64>(4)?,
                ))
            },
        )?;
        let mut entries = Vec::new();
        let mut bytes = 0;
        let mut more = false;
        let mut oldest = None;
        for row in rows {
            let (sequence, sample_revision, captured_unix_ms, data, position) = row?;
            if entries.len() >= 32 || (!entries.is_empty() && bytes + data.len() > 8 * 1024 * 1024)
            {
                more = true;
                break;
            }
            bytes += data.len();
            oldest = Some(position);
            entries.push(ArchivedMessage {
                sequence,
                sample_revision,
                captured_unix_ms,
                message: serde_json::from_str(&data)?,
            });
        }
        entries.reverse();
        Ok(ArchiveWindow {
            status: status.map(|s| serde_json::from_str(&s)).transpose()?,
            entries,
            next_after: latest_sequence,
            next_before: if more { oldest } else { None },
            latest_sequence,
            more_newer: false,
        })
    }
    pub fn event_payload(&self, id: u64) -> Result<FactoryEvent> {
        if id == 0 || id > i64::MAX as u64 {
            return Err(StoreError::Invalid("Invalid event ID"));
        }
        let page = self.events(id - 1, 1)?;
        page.events
            .into_iter()
            .find(|e| e.sequence == id)
            .ok_or(StoreError::NotFound)
    }
    pub fn event_headers(&self, after: u64, limit: u32) -> Result<EventPage> {
        if after > i64::MAX as u64 || !(1..=500).contains(&limit) {
            return Err(StoreError::Invalid("Invalid event cursor or limit"));
        }
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let mut stmt = db.prepare("SELECT sequence,kind,aggregate_id,created_unix_ms FROM events WHERE sequence>?1 ORDER BY sequence LIMIT ?2")?;
        let rows = stmt.query_map(params![after, limit], |r| {
            Ok(FactoryEvent {
                sequence: r.get(0)?,
                kind: r.get(1)?,
                aggregate_id: r.get(2)?,
                created_unix_ms: r.get(3)?,
                payload: serde_json::Value::Null,
            })
        })?;
        let events = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        let next_cursor = events.last().map_or(after, |e| e.sequence);
        Ok(EventPage {
            events,
            next_cursor,
        })
    }
    pub fn desktop_events(&self, after: u64) -> Result<EventPage> {
        if after > i64::MAX as u64 {
            return Err(StoreError::Invalid("Invalid event cursor"));
        }
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let mut stmt = db.prepare("SELECT sequence,kind,aggregate_id,created_unix_ms,payload FROM events WHERE sequence>?1 ORDER BY sequence LIMIT 100")?;
        let rows = stmt.query_map([after], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (sequence, kind, aggregate_id, created_unix_ms, payload) = row?;
            let mut payload = serde_json::from_str(&payload)?;
            defer_binary(&mut payload);
            events.push(FactoryEvent {
                sequence,
                kind,
                aggregate_id,
                created_unix_ms,
                payload,
            });
        }
        let next_cursor = events.last().map_or(after, |e| e.sequence);
        Ok(EventPage {
            events,
            next_cursor,
        })
    }
    pub fn task_payload(&self, id: TicketId) -> Result<Ticket> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let data: Option<String> = db
            .query_row(
                "SELECT t.data FROM tickets t WHERE t.id=?1 AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?)
    }
    pub fn command_payload(&self, id: CommandId) -> Result<RunnerCommand> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let data: Option<String> = db
            .query_row(
                "SELECT data FROM commands WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?)
    }
    pub fn notification_states(
        &self,
        owner: &str,
        ids: &[u64],
    ) -> Result<Vec<NotificationReadState>> {
        if ids.len() > 1000 || ids.iter().any(|id| *id > i64::MAX as u64) {
            return Err(StoreError::Invalid("Too many or invalid notification IDs"));
        }
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let mut stmt = db.prepare(
            "SELECT EXISTS(SELECT 1 FROM notification_reads WHERE owner=?1 AND notification_id=?2)",
        )?;
        ids.iter()
            .map(|id| {
                Ok(NotificationReadState {
                    id: *id,
                    read: stmt.query_row(params![owner, id], |r| r.get(0))?,
                })
            })
            .collect()
    }
    /// Current window first, then cursor-based history or live catch-up. Never truncate a message.
    pub fn archive_window(
        &self,
        id: CommandId,
        after: Option<u64>,
        before: Option<u64>,
    ) -> Result<ArchiveWindow> {
        if (after.is_some() && before.is_some())
            || after.into_iter().chain(before).any(|v| v > i64::MAX as u64)
        {
            return Err(StoreError::Invalid("Invalid archive window cursor"));
        }
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
        let status: Option<String> = tx
            .query_row(
                "SELECT data FROM archive_status WHERE command_id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let status = status.map(|s| serde_json::from_str(&s)).transpose()?;
        let latest_sequence = tx.query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM archive_messages WHERE command_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        let (sql, cursor) = if let Some(after) = after {
            ("SELECT sequence,sample_revision,captured_unix_ms,data FROM archive_messages WHERE command_id=?1 AND sequence>?2 ORDER BY sequence ASC LIMIT 33", after)
        } else {
            ("SELECT sequence,sample_revision,captured_unix_ms,data FROM archive_messages WHERE command_id=?1 AND sequence<?2 ORDER BY sequence DESC LIMIT 33", before.unwrap_or(i64::MAX as u64))
        };
        let mut stmt = tx.prepare(sql)?;
        let rows = stmt.query_map(params![id.to_string(), cursor], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut entries = Vec::new();
        let mut bytes = 0;
        let mut more = false;
        for row in rows {
            let (sequence, sample_revision, captured_unix_ms, data) = row?;
            if entries.len() >= 32 || (!entries.is_empty() && bytes + data.len() > 8 * 1024 * 1024)
            {
                more = true;
                break;
            }
            bytes += data.len();
            entries.push(ArchivedMessage {
                sequence,
                sample_revision,
                captured_unix_ms,
                message: serde_json::from_str(&data)?,
            });
        }
        if after.is_none() {
            entries.reverse();
        }
        let next_after = after.map_or(latest_sequence, |cursor| {
            entries.last().map_or(cursor, |e| e.sequence)
        });
        let next_before = if after.is_none() && more {
            entries.first().map(|e| e.sequence)
        } else {
            None
        };
        Ok(ArchiveWindow {
            status,
            entries,
            next_after,
            next_before,
            latest_sequence,
            more_newer: after.is_some() && more,
        })
    }
    pub fn validate_instance_commands(&self, runner: RunnerId, ids: &[CommandId]) -> Result<()> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        for id in ids {
            let owner: Option<String> = db
                .query_row(
                    "SELECT runner_id FROM commands WHERE id=?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if owner.as_deref() != Some(&runner.to_string()) {
                return Err(StoreError::WorkConflict);
            }
        }
        Ok(())
    }
    pub fn event_history(&self, before: Option<u64>) -> Result<EventHistoryPage> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let mut stmt = db.prepare("SELECT sequence,kind,aggregate_id,created_unix_ms,payload FROM events WHERE sequence<?1 ORDER BY sequence DESC LIMIT 101")?;
        let rows = stmt.query_map(
            [before.unwrap_or(i64::MAX as u64).min(i64::MAX as u64)],
            |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )?;
        let mut events = Vec::new();
        for row in rows {
            let (sequence, kind, aggregate_id, created_unix_ms, payload) = row?;
            let mut payload = serde_json::from_str(&payload)?;
            defer_binary(&mut payload);
            events.push(FactoryEvent {
                sequence,
                kind,
                aggregate_id,
                created_unix_ms,
                payload,
            });
        }
        let more = events.len() > 100;
        events.truncate(100);
        let next_before = if more {
            events.last().map(|e| e.sequence)
        } else {
            None
        };
        events.reverse();
        Ok(EventHistoryPage {
            events,
            next_before,
        })
    }
    pub fn notifications(&self, owner: &str, before: Option<u64>) -> Result<NotificationPage> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let ceiling = before.unwrap_or(i64::MAX as u64).min(i64::MAX as u64);
        let mut stmt = db.prepare("SELECT n.data,EXISTS(SELECT 1 FROM notification_reads r WHERE r.owner=?1 AND r.notification_id=n.id) FROM notifications n WHERE n.id<?2 ORDER BY n.id DESC LIMIT 101")?;
        let rows = stmt.query_map(params![owner, ceiling], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (data, read) = row?;
            let mut n: Notification = serde_json::from_str(&data)?;
            n.read = read;
            entries.push(n);
        }
        let more = entries.len() > 100;
        entries.truncate(100);
        let unread = db.query_row("SELECT COUNT(*) FROM notifications n WHERE NOT EXISTS(SELECT 1 FROM notification_reads r WHERE r.owner=?1 AND r.notification_id=n.id)", [owner], |r| r.get(0))?;
        let next_before = if more {
            entries.last().map(|n| n.id)
        } else {
            None
        };
        Ok(NotificationPage {
            entries,
            unread,
            next_before,
        })
    }
    pub fn mark_notifications(&self, owner: &str, body: MarkNotifications) -> Result<()> {
        if body.ids.len() > 1000 {
            return Err(StoreError::Invalid("Too many notification IDs"));
        }
        self.mutate(body.request_id, &format!("notifications.read/{owner}"), &body, |tx| {
            for id in &body.ids {
                if body.read { tx.execute("INSERT OR IGNORE INTO notification_reads(owner,notification_id) SELECT ?1,id FROM notifications WHERE id=?2", params![owner, id])?; }
                else { tx.execute("DELETE FROM notification_reads WHERE owner=?1 AND notification_id=?2", params![owner, id])?; }
            }
            Ok(())
        })
    }
}
