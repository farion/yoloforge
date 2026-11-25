use super::*;

impl Store {
    /// Coalesce pending human questions into one wakeup. Lease/retry state is durable
    /// before any HTTP dispatch, including crashes and uncertain provider responses.
    pub fn claim_push(&self, destination: &str, now: u64) -> Result<Option<u64>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO push_delivery(id,destination) VALUES(1,?1) ON CONFLICT(id) DO NOTHING",
            [destination],
        )?;
        let (bound, cursor, pending, attempts, next): (String, u64, u64, u32, u64) = tx.query_row(
            "SELECT destination,cursor,pending,attempts,next_attempt FROM push_delivery WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        // A new operator-configured destination starts a fresh delivery scope. No secrets in DB/events.
        if bound != destination {
            tx.execute("UPDATE push_delivery SET destination=?1,cursor=0,pending=0,attempts=0,next_attempt=0 WHERE id=1",[destination])?;
            tx.commit()?;
            return Ok(None);
        }
        if now < next {
            return Ok(None);
        }
        let latest:u64=tx.query_row("SELECT COALESCE(MAX(e.sequence),0) FROM events e JOIN questions q ON q.id=e.aggregate_id WHERE e.kind IN ('question.created','question.updated') AND e.sequence>?1 AND json_extract(q.data,'$.state') IN ('pending','rejected') AND NOT EXISTS(SELECT 1 FROM commands c JOIN directs d ON d.stream_id=c.stream_id WHERE c.id=q.command_id)",[cursor],|r|r.get(0))?;
        if latest == 0 {
            // An answered question no longer needs a notification, even after an uncertain POST.
            tx.execute("UPDATE push_delivery SET cursor=MAX(cursor,pending),pending=0,attempts=0,next_attempt=0 WHERE id=1",[])?;
            tx.commit()?;
            return Ok(None);
        }
        let sequence = pending.max(latest);
        let delay = 30_000u64
            .saturating_mul(1u64 << attempts.min(5))
            .min(900_000);
        tx.execute("UPDATE push_delivery SET pending=?1,attempts=MIN(attempts+1,32),next_attempt=?2 WHERE id=1",params![sequence,now.saturating_add(delay)])?;
        tx.commit()?;
        Ok(Some(sequence))
    }
    pub fn confirm_push(&self, destination: &str, sequence: u64, now: u64) -> Result<()> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        db.execute("UPDATE push_delivery SET cursor=?1,pending=0,attempts=0,next_attempt=?2 WHERE id=1 AND destination=?3 AND pending=?1",params![sequence,now.saturating_add(30_000),destination])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wakeups_coalesce_survive_restart_back_off_and_fence_old_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        // Minimal foreign-key parents; delivery only inspects the durable question state.
        {
            let db = store.0.lock().unwrap();
            db.execute_batch("PRAGMA foreign_keys=OFF; INSERT INTO questions(id,command_id,kind,external_id,data) VALUES('q','c','form','f','{\"state\":\"pending\"}'); INSERT INTO events(kind,aggregate_id,created_unix_ms,payload) VALUES('question.created','q',1,'{}'); PRAGMA foreign_keys=ON;").unwrap();
        }
        let sequence = store.claim_push("first", 1000).unwrap().unwrap();
        assert!(store.claim_push("first", 1001).unwrap().is_none());
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(store.claim_push("first", 30_999).unwrap().is_none());
        assert_eq!(store.claim_push("first", 31_000).unwrap(), Some(sequence));
        assert!(store.claim_push("first", 60_999).unwrap().is_none());
        store.confirm_push("first", sequence, 32_000).unwrap();
        assert!(store.claim_push("first", 100_000).unwrap().is_none());
        assert!(store.claim_push("second", 100_001).unwrap().is_none());
        let second = store.claim_push("second", 100_002).unwrap().unwrap();
        store.confirm_push("first", second, 100_003).unwrap();
        assert!(store.claim_push("second", 100_004).unwrap().is_none());
        {
            let db = store.0.lock().unwrap();
            db.execute("UPDATE questions SET data='{\"state\":\"answered\"}'", [])
                .unwrap();
        }
        assert!(store.claim_push("second", 999_999).unwrap().is_none());
    }
}
