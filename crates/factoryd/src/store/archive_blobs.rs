use super::*;

fn hash(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(StoreError::Invalid("Invalid archive blob digest"));
    }
    Ok(())
}
impl Store {
    pub fn report_archive_blob(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: ReportArchiveBlob,
    ) -> Result<ArchiveBlobProgress> {
        let chunk = body.chunk;
        let file = &chunk.file;
        hash(&file.sha256)?;
        for (value, max) in [
            (&file.uri, 8192),
            (&file.name, 1024),
            (&file.mime, 200),
            (&file.message_id, 200),
        ] {
            line(value, max)?;
        }
        if file.bytes == 0
            || file.bytes > ARCHIVE_BLOB_LIMIT
            || file.source_revision == 0
            || file.source_revision > i64::MAX as u64
            || !chunk.offset.is_multiple_of(ARCHIVE_CHUNK_LIMIT as u64)
        {
            return Err(StoreError::Invalid("Invalid archive blob bounds"));
        }
        let bytes = chunk.bytes().map_err(StoreError::Invalid)?;
        let end = chunk
            .offset
            .checked_add(bytes.len() as u64)
            .ok_or(StoreError::Invalid("Invalid chunk offset"))?;
        if end > file.bytes || (end != file.bytes && bytes.len() != ARCHIVE_CHUNK_LIMIT) {
            return Err(StoreError::Invalid("Invalid chunk boundary"));
        }
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let command = super::work::command(&tx, id, runner, body.ownership_epoch)?;
        if command.kind != CommandKind::RunAgent
            || command.state == CommandState::Pending
            || super::termination::load(&tx, id)?
                .is_some_and(|v| v.state == TerminationState::Stopped)
        {
            return Err(StoreError::WorkConflict);
        }
        let known_revision: Option<u64> = tx
            .query_row(
                "SELECT json_extract(data,'$.revision') FROM archive_status WHERE command_id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if known_revision.is_none_or(|v| file.source_revision > v) {
            return Err(StoreError::WorkConflict);
        }
        // Supplemental bytes must correspond to an already archived typed URI,
        // never an arbitrary file claimed independently of the conversation.
        let raw:Option<String>=tx.query_row("SELECT data FROM archive_messages WHERE command_id=?1 AND message_id=?2 AND sample_revision<=?3 ORDER BY sequence DESC LIMIT 1",params![id.to_string(),file.message_id,file.source_revision],|r|r.get(0)).optional()?;
        let raw: serde_json::Value = serde_json::from_str(&raw.ok_or(StoreError::WorkConflict)?)?;
        if !archive_file_references(&raw)
            .iter()
            .any(|(uri, _, _)| uri == &file.uri)
        {
            return Err(StoreError::WorkConflict);
        }
        let previous:Option<(String,u64,bool)>=tx.query_row("SELECT data,received,complete FROM archive_blobs WHERE command_id=?1 AND sha256=?2",params![id.to_string(),file.sha256],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (received, complete) = if let Some((data, received, complete)) = previous {
            let saved: ArchiveBlob = serde_json::from_str(&data)?;
            if saved != *file {
                return Err(StoreError::RequestConflict);
            }
            (received, complete)
        } else {
            let (count, total): (u64, u64) = tx.query_row(
                "SELECT count(*),coalesce(sum(bytes),0) FROM archive_blobs WHERE command_id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if count >= 16 || total + file.bytes > 256 * 1024 * 1024 {
                return Err(StoreError::Invalid("Command archive blob budget exceeded"));
            }
            tx.execute(
                "INSERT INTO archive_blobs(command_id,sha256,data,bytes) VALUES(?1,?2,?3,?4)",
                params![
                    id.to_string(),
                    file.sha256,
                    serde_json::to_string(file)?,
                    file.bytes
                ],
            )?;
            (0, false)
        };
        if chunk.offset < received {
            let previous:Option<Vec<u8>>=tx.query_row("SELECT data FROM archive_blob_chunks WHERE command_id=?1 AND sha256=?2 AND offset=?3",params![id.to_string(),file.sha256,chunk.offset],|r|r.get(0)).optional()?;
            if previous.as_ref() != Some(&bytes) {
                return Err(StoreError::RequestConflict);
            }
            return Ok(ArchiveBlobProgress {
                next_offset: received,
                complete,
            });
        }
        if chunk.offset != received || complete {
            return Err(StoreError::WorkConflict);
        }
        tx.execute(
            "INSERT INTO archive_blob_chunks(command_id,sha256,offset,data) VALUES(?1,?2,?3,?4)",
            params![id.to_string(), file.sha256, chunk.offset, bytes],
        )?;
        let complete = end == file.bytes;
        if complete {
            let mut digest = Sha256::new();
            let mut statement=tx.prepare("SELECT data FROM archive_blob_chunks WHERE command_id=?1 AND sha256=?2 ORDER BY offset")?;
            let rows = statement.query_map(params![id.to_string(), file.sha256], |r| {
                r.get::<_, Vec<u8>>(0)
            })?;
            for row in rows {
                digest.update(row?);
            }
            if format!("{:x}", digest.finalize()) != file.sha256 {
                return Err(StoreError::Invalid("Archive blob digest mismatch"));
            }
        }
        tx.execute(
            "UPDATE archive_blobs SET received=?3,complete=?4 WHERE command_id=?1 AND sha256=?2",
            params![id.to_string(), file.sha256, end, complete],
        )?;
        if complete {
            event(&tx, "archive.file", id.to_string(), file)?;
        }
        tx.commit()?;
        Ok(ArchiveBlobProgress {
            next_offset: end,
            complete,
        })
    }

    pub fn archive_blobs(&self, id: CommandId, through: Option<u64>) -> Result<ArchiveBlobIndex> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let mut stmt = db.prepare(
            "SELECT data FROM archive_blobs WHERE command_id=?1 AND complete=1 ORDER BY rowid",
        )?;
        let rows = stmt.query_map([id.to_string()], |r| r.get::<_, String>(0))?;
        let mut files = Vec::new();
        for row in rows {
            let file: ArchiveBlob = serde_json::from_str(&row?)?;
            if through.is_none_or(|v| file.source_revision <= v) {
                files.push(file);
            }
        }
        Ok(ArchiveBlobIndex { files })
    }
    pub fn archive_blob_chunk(
        &self,
        id: CommandId,
        sha256: &str,
        offset: u64,
    ) -> Result<ArchiveBlobChunk> {
        hash(sha256)?;
        if offset > ARCHIVE_BLOB_LIMIT || !offset.is_multiple_of(ARCHIVE_CHUNK_LIMIT as u64) {
            return Err(StoreError::Invalid("Invalid chunk offset"));
        }
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let value:Option<(String,Vec<u8>)>=db.query_row("SELECT b.data,c.data FROM archive_blobs b JOIN archive_blob_chunks c ON c.command_id=b.command_id AND c.sha256=b.sha256 WHERE b.command_id=?1 AND b.sha256=?2 AND b.complete=1 AND c.offset=?3",params![id.to_string(),sha256,offset],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (file, bytes) = value.ok_or(StoreError::NotFound)?;
        Ok(ArchiveBlobChunk::new(
            serde_json::from_str(&file)?,
            offset,
            &bytes,
        ))
    }
}
