use super::*;
use crate::android_push::{Config, Entry, Registration, SettingsUpdate, Target};

impl Store {
    pub(crate) fn android_push_config(&self) -> Result<Option<Config>> {
        match std::fs::read(self.2.as_ref()) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(StoreError::Internal),
        }
    }

    pub(crate) fn save_android_push(&self, update: SettingsUpdate) -> Result<serde_json::Value> {
        let _db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let config = update.apply(self.android_push_config()?)?;
        use std::io::Write;
        let parent = self
            .2
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| StoreError::Internal)?;
        file.write_all(&serde_json::to_vec(&config)?)
            .map_err(|_| StoreError::Internal)?;
        file.as_file()
            .sync_all()
            .map_err(|_| StoreError::Internal)?;
        file.persist(self.2.as_ref())
            .map_err(|_| StoreError::Internal)?;
        Ok(config.status())
    }

    pub(crate) fn register_android_push(
        &self,
        id: &str,
        owner: &str,
        input: Registration,
    ) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let config = self
            .android_push_config()?
            .filter(|c| c.enabled)
            .ok_or(StoreError::NotFound)?;
        if input.sender != config.sender()
            || input.token.is_empty()
            || input.token.len() > 4096
            || !input.token.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(StoreError::Invalid("Invalid Android push registration"));
        }
        let tx = db.transaction()?;
        // Same app installation may return the same FCM token after a local re-login.
        tx.execute(
            "DELETE FROM android_push_devices WHERE token=?1 AND owner=?2 AND id<>?3",
            params![input.token, owner, id],
        )?;
        let count = tx
            .execute(
                "INSERT INTO android_push_devices(id,owner,sender,token) VALUES(?1,?2,?3,?4)
             ON CONFLICT(id) DO UPDATE SET sender=excluded.sender,token=excluded.token,retry_after=0
             WHERE owner=excluded.owner AND (sender<>excluded.sender OR token<>excluded.token)",
                params![id, owner, input.sender, input.token],
            )
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(code, _)
                    if code.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    StoreError::WorkConflict
                }
                _ => StoreError::Internal,
            })?;
        if count == 0
            && !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM android_push_devices WHERE id=?1 AND owner=?2)",
                params![id, owner],
                |r| r.get(0),
            )?
        {
            return Err(StoreError::NotFound);
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn unregister_android_push(&self, id: &str, owner: &str) -> Result<()> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        db.execute(
            "DELETE FROM android_push_devices WHERE id=?1 AND owner=?2",
            params![id, owner],
        )?;
        Ok(())
    }

    pub(crate) fn android_push_batch(
        &self,
        sender: &str,
        bootstrap: &str,
        now: u64,
    ) -> Result<Vec<(Target, Entry)>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        tx.execute(
            "DELETE FROM android_push_devices WHERE (owner<>?1 OR owner='') AND NOT EXISTS
            (SELECT 1 FROM devices WHERE id=owner AND revoked=0)",
            [bootstrap],
        )?;
        let targets = {
            let mut stmt = tx.prepare("SELECT id,token FROM android_push_devices WHERE sender=?1 AND retry_after<=?2 ORDER BY rowid")?;
            let rows = stmt.query_map(params![sender, now], |r| {
                Ok(Target {
                    id: r.get(0)?,
                    token: r.get(1)?,
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if targets.is_empty() {
            tx.commit()?;
            return Ok(Vec::new());
        }
        let questions: Vec<Question> = records(
            &tx,
            "SELECT q.data FROM questions q JOIN commands c ON c.id=q.command_id
             JOIN streams s ON s.id=c.stream_id JOIN active_projects p ON p.id=s.project_id
             WHERE json_extract(q.data,'$.state') IN ('pending','rejected')
             AND NOT EXISTS(SELECT 1 FROM directs d WHERE d.stream_id=c.stream_id)
             ORDER BY q.rowid",
        )?;
        let mut entries: Vec<Entry> = questions
            .into_iter()
            .filter(|q| {
                q.state != QuestionState::Pending
                    || q.orchestration_note != QUESTION_AUTOMATION_PENDING
            })
            .map(|q| Entry {
                key: format!("question/{}/{}", q.id, q.update_seq.0),
                kind: "question",
                id: q.id.to_string(),
            })
            .collect();
        let problems = desktop::intent_problems(&tx)?;
        let resets: Vec<DraftReset> =
            records(&tx, "SELECT data FROM draft_resets WHERE completed=0")?;
        for problem in problems {
            let ticket: Option<String> = tx.query_row(
                "SELECT t.data FROM tickets t JOIN active_projects p ON p.id=t.project_id
                 WHERE t.id=?1 AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)",
                [problem.ticket_id.to_string()], |r| r.get(0)).optional()?;
            let Some(ticket) = ticket else { continue };
            let t: Ticket = serde_json::from_str(&ticket)?;
            if !t.draft
                && !delivery_retry_pending(&t)
                && !resets
                    .iter()
                    .any(|r| r.ticket_id == t.id && r.command_id == problem.command_id)
                && (matches!(t.status, TicketState::Blocked | TicketState::InProgress)
                    && t.execution_command == Some(problem.command_id)
                    || problem.solution == Some(IntentSolution::ReassignSession)
                        && matches!(t.status, TicketState::Backlog | TicketState::Ready)
                        && t.execution_command.is_none()
                        && t.requested_session == Some(problem.command_id))
            {
                entries.push(Entry {
                    key: format!("problem/{}/{}", t.id, problem.command_id),
                    kind: "problem",
                    id: t.id.to_string(),
                });
            }
        }
        let mut batch = Vec::new();
        {
            let mut delivered = tx.prepare(
                "SELECT EXISTS(SELECT 1 FROM android_push_delivered WHERE device=?1 AND entry=?2)",
            )?;
            for target in targets {
                for entry in &entries {
                    if !delivered
                        .query_row(params![target.id, entry.key], |r| r.get::<_, bool>(0))?
                    {
                        batch.push((target, entry.clone()));
                        break;
                    }
                }
                if batch.len() >= 32 {
                    break;
                }
            }
        }
        tx.commit()?;
        Ok(batch)
    }

    pub(crate) fn android_push_result(
        &self,
        target: Target,
        entry: Entry,
        result: crate::android_push::Delivery,
    ) -> Result<()> {
        use crate::android_push::Delivery;
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        match result {
            Delivery::Sent => db.execute(
                "INSERT OR IGNORE INTO android_push_delivered(device,entry)
                 SELECT id,?3 FROM android_push_devices WHERE id=?1 AND token=?2",
                params![target.id, target.token, entry.key],
            )?,
            Delivery::Unregistered => db.execute(
                "DELETE FROM android_push_devices WHERE id=?1 AND token=?2",
                params![target.id, target.token],
            )?,
            Delivery::Retry => db.execute(
                "UPDATE android_push_devices SET retry_after=?3 WHERE id=?1 AND token=?2",
                params![target.id, target.token, super::super::unix_ms() + 60_000],
            )?,
        };
        Ok(())
    }
}
