use super::*;
impl Store {
    pub fn report_archive(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: ReportArchive,
    ) -> Result<ArchiveStatus> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let command = super::work::command(&tx, id, runner, body.ownership_epoch)?;
        if super::termination::load(&tx, id)?.is_some_and(|t| t.state == TerminationState::Stopped)
        {
            return Err(StoreError::WorkConflict);
        }
        let batch = body.batch;
        if command.kind != CommandKind::RunAgent
            || command.state == CommandState::Pending
            || batch.session_id != command.session_id()
        {
            return Err(StoreError::WorkConflict);
        }
        let encoded = serde_json::to_vec(&batch)?;
        if encoded.len() > 8 * 1024 * 1024
            || batch.messages.len() > 16
            || batch.revision == 0
            || batch.revision > i64::MAX as u64
            || batch.captured_unix_ms > i64::MAX as u64
        {
            return Err(StoreError::Invalid("Invalid archive batch"));
        }
        let fingerprint = format!("{:x}", Sha256::digest(&encoded));
        let previous: Option<(String, String)> = tx
            .query_row(
                "SELECT fingerprint,data FROM archive_status WHERE command_id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let previous = if let Some((hash, data)) = previous {
            let status: ArchiveStatus = serde_json::from_str(&data)?;
            if status.revision == batch.revision && hash == fingerprint {
                return Ok(status);
            }
            Some(status)
        } else {
            None
        };
        if batch.revision != previous.as_ref().map_or(1, |v| v.revision + 1)
            || previous.as_ref().is_some_and(|v| v.final_scan_complete)
        {
            return Err(StoreError::WorkConflict);
        }
        let mut seen = std::collections::HashSet::new();
        for message in &batch.messages {
            let message_id = message["id"]
                .as_str()
                .ok_or(StoreError::Invalid("Archive message ID missing"))?;
            line(message_id, 200)?;
            if !seen.insert(message_id)
                || message
                    .get("sessionID")
                    .is_some_and(|v| v != &batch.session_id)
            {
                return Err(StoreError::Invalid("Foreign or duplicate archive message"));
            }
            let data = serde_json::to_string(message)?;
            let hash = format!("{:x}", Sha256::digest(data.as_bytes()));
            let latest:Option<String>=tx.query_row("SELECT fingerprint FROM archive_messages WHERE command_id=?1 AND message_id=?2 ORDER BY sequence DESC LIMIT 1",params![id.to_string(),message_id],|r|r.get(0)).optional()?;
            if latest.as_ref() != Some(&hash) {
                tx.execute("INSERT INTO archive_messages(command_id,message_id,fingerprint,sample_revision,captured_unix_ms,data) VALUES(?1,?2,?3,?4,?5,?6)",params![id.to_string(),message_id,hash,batch.revision,batch.captured_unix_ms,data])?;
            }
        }
        let status = ArchiveStatus {
            session_id: batch.session_id,
            revision: batch.revision,
            captured_unix_ms: batch.captured_unix_ms,
            final_scan_complete: batch.final_scan_complete,
        };
        tx.execute("INSERT INTO archive_status(command_id,fingerprint,data) VALUES(?1,?2,?3) ON CONFLICT(command_id) DO UPDATE SET fingerprint=excluded.fingerprint,data=excluded.data",params![id.to_string(),fingerprint,serde_json::to_string(&status)?])?;
        tx.commit()?;
        Ok(status)
    }
    pub fn archive(&self, id: CommandId, after: u64, through: Option<u64>) -> Result<ArchivePage> {
        if after > i64::MAX as u64 {
            return Err(StoreError::Invalid("Invalid archive cursor"));
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
        let status: Option<ArchiveStatus> = status.map(|v| serde_json::from_str(&v)).transpose()?;
        let ceiling = through.unwrap_or_else(|| status.as_ref().map_or(0, |s| s.revision));
        if ceiling > i64::MAX as u64 {
            return Err(StoreError::Invalid("Invalid archive revision"));
        }
        // A page contains at most one raw message (each <=8 MiB), keeping reads bounded.
        let mut stmt=tx.prepare("SELECT sequence,sample_revision,captured_unix_ms,data FROM archive_messages WHERE command_id=?1 AND sequence>?2 AND sample_revision<=?3 ORDER BY sequence LIMIT 1")?;
        let rows = stmt.query_map(params![id.to_string(), after, ceiling], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut page = ArchivePage {
            termination: super::termination::load(&tx, id)?,
            status,
            entries: Vec::new(),
            next_after: after,
        };
        for row in rows {
            let (sequence, sample_revision, captured_unix_ms, data) = row?;
            page.next_after = sequence;
            page.entries.push(ArchivedMessage {
                sequence,
                sample_revision,
                captured_unix_ms,
                message: serde_json::from_str(&data)?,
            });
        }
        Ok(page)
    }
}
