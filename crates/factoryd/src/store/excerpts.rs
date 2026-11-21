use super::*;
impl Store {
    pub fn excerpt(&self, id: CommandId) -> Result<Option<SessionExcerpt>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands WHERE id=?1)",
            [id.to_string()],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound);
        }
        let data: Option<String> = db
            .query_row(
                "SELECT data FROM excerpts WHERE command_id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }
    pub fn report_excerpt(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: ReportExcerpt,
    ) -> Result<SessionExcerpt> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let command = super::work::command(&tx, id, runner, body.ownership_epoch)?;
        let value = body.excerpt;
        if command.kind != CommandKind::RunAgent || command.state == CommandState::Pending {
            return Err(StoreError::WorkConflict);
        }
        if value.session_id != command.session_id()
            || value.revision.0 == 0
            || value.messages.len() > 32
        {
            return Err(StoreError::Invalid("Invalid session excerpt"));
        }
        let mut ids = std::collections::HashSet::new();
        let mut size = 0;
        for m in &value.messages {
            line(&m.id, 200)?;
            line(&m.kind, 80)?;
            if !ids.insert(&m.id) || m.text.len() > 8192 {
                return Err(StoreError::Invalid("Invalid excerpt message"));
            }
            size += m.text.len();
        }
        if size > 131072 {
            return Err(StoreError::Invalid("Session excerpt too large"));
        }
        let old: Option<String> = tx
            .query_row(
                "SELECT data FROM excerpts WHERE command_id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(old) = old {
            let old: SessionExcerpt = serde_json::from_str(&old)?;
            if value == old {
                return Ok(old);
            }
            if value.revision.0 <= old.revision.0 {
                return Err(StoreError::WorkConflict);
            }
        }
        tx.execute("INSERT INTO excerpts(command_id,data) VALUES(?1,?2) ON CONFLICT(command_id) DO UPDATE SET data=excluded.data",params![id.to_string(),serde_json::to_string(&value)?])?;
        tx.commit()?;
        Ok(value)
    }
}
