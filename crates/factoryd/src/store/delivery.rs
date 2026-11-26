//! Durable retry pacing. No retry count turns a technical failure into a terminal intent.
use super::*;

pub(super) fn ready(tx: &Connection, ticket: TicketId, step: &str) -> Result<bool> {
    Ok(!tx.query_row("SELECT EXISTS(SELECT 1 FROM delivery_retries WHERE ticket_id=?1 AND step=?2 AND next_ms>?3)",
        params![ticket.to_string(), step, super::super::unix_ms()], |r| r.get::<_, bool>(0))?)
}

pub(super) fn defer(tx: &Connection, ticket: TicketId, step: &str) -> Result<()> {
    let attempt: Option<u32> = tx.query_row(
        "SELECT attempt FROM delivery_retries WHERE ticket_id=?1 AND step=?2",
        params![ticket.to_string(), step], |r| r.get(0)).optional()?;
    let attempt = attempt.unwrap_or(0).saturating_add(1);
    let delay = (15_000_u64 << attempt.saturating_sub(1).min(6)).min(900_000);
    tx.execute("INSERT INTO delivery_retries VALUES(?1,?2,?3,?4)
        ON CONFLICT(ticket_id,step) DO UPDATE SET attempt=excluded.attempt,next_ms=excluded.next_ms",
        params![ticket.to_string(), step, attempt, super::super::unix_ms().saturating_add(delay)])?;
    Ok(())
}

pub(super) fn clear(tx: &Connection, ticket: TicketId, step: &str) -> Result<()> {
    tx.execute("DELETE FROM delivery_retries WHERE ticket_id=?1 AND step=?2", params![ticket.to_string(), step])?;
    Ok(())
}
