//! Fence new acknowledgements while a desktop replaces an idle local runner.
use super::*;

impl Store {
    pub fn hold_runner_restart(&self, runner: RunnerId, request: RequestId) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = super::super::unix_ms();
        let held: Option<String> = tx
            .query_row(
                "SELECT request_id FROM runner_restart_holds WHERE runner_id=?1 AND expires_ms>?2",
                params![runner.to_string(), now],
                |r| r.get(0),
            )
            .optional()?;
        if held.is_some_and(|owner| owner != request.to_string()) {
            return Err(StoreError::WorkConflict);
        }
        // Queued work is retained and may resume after restart. Accepted or
        // ambiguous work must never be interrupted, even if the GUI was stale.
        let blocked: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands WHERE runner_id=?1 AND
                (state='accepted' OR COALESCE(json_extract(data,'$.result.agent.phase'),
                    json_extract(data,'$.agent_progress.phase'))='unknown'))
             OR EXISTS(SELECT 1 FROM streams WHERE json_extract(data,'$.runner_id')=?1 AND
                json_extract(data,'$.status') IN ('running','waiting_input','pausing','paused'))
             OR EXISTS(SELECT 1 FROM controls c JOIN streams s ON s.id=c.stream_id
                WHERE json_extract(s.data,'$.runner_id')=?1 AND json_extract(c.data,'$.state')<>'released')
             OR EXISTS(SELECT 1 FROM runtime_terminations r JOIN commands c ON c.id=r.command_id
                WHERE c.runner_id=?1 AND json_extract(r.data,'$.state')<>'stopped')
              OR EXISTS(SELECT 1 FROM publication_recovery_holds WHERE runner_id=?1 AND expires_ms>?2)",
            params![runner.to_string(), now], |r| r.get(0),
        )?;
        if blocked {
            return Err(StoreError::Invalid(
                "Runner hat laufende oder ungeklärte Aufträge; Neustart nicht sicher",
            ));
        }
        tx.execute("INSERT INTO runner_restart_holds(runner_id,request_id,expires_ms) VALUES(?1,?2,?3)
            ON CONFLICT(runner_id) DO UPDATE SET request_id=excluded.request_id,expires_ms=excluded.expires_ms",
            params![runner.to_string(), request.to_string(), now.saturating_add(180_000)])?;
        tx.commit()?;
        Ok(())
    }

    pub fn release_runner_restart(&self, runner: RunnerId, request: RequestId) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let held: Option<String> = tx
            .query_row(
                "SELECT request_id FROM runner_restart_holds WHERE runner_id=?1",
                [runner.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if held.is_some_and(|owner| owner != request.to_string()) {
            return Err(StoreError::WorkConflict);
        }
        tx.execute(
            "DELETE FROM runner_restart_holds WHERE runner_id=?1 AND request_id=?2",
            params![runner.to_string(), request.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
}
