use super::*;

pub(super) fn project_key(
    tx: &Transaction<'_>, name: &str, requested: Option<&str>, own: Option<ProjectId>,
) -> Result<String> {
    let explicit = requested.map(str::trim).filter(|key| !key.is_empty());
    let base = if let Some(key) = explicit {
        let key = key.to_ascii_uppercase();
        if key.len() > 10 || !key.bytes().next().is_some_and(|c| c.is_ascii_uppercase())
            || !key.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err(StoreError::Invalid("Projektkürzel: 1–10 Buchstaben/Ziffern, beginnend mit einem Buchstaben"));
        }
        key
    } else {
        let capitals: String = name.chars().filter(|c| c.is_ascii_uppercase()).take(10).collect();
        let words: Vec<_> = name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|word| !word.is_empty()).collect();
        let mut key: String = if capitals.len() >= 2 { capitals }
            else if words.len() >= 2 { words.iter().filter_map(|word| word.chars().next()).take(10).collect::<String>().to_ascii_uppercase() }
            else { name.chars().filter(char::is_ascii_alphanumeric).take(10).collect::<String>().to_ascii_uppercase() };
        if !key.bytes().next().is_some_and(|c| c.is_ascii_uppercase()) { key.insert(0,'P'); }
        key.truncate(10);
        key
    };
    for suffix in 0..1_000_000 {
        let key = if suffix == 0 { base.clone() } else {
            let tail = (suffix+1).to_string();
            format!("{}{}", &base[..base.len().min(10-tail.len())], tail)
        };
        let taken: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE json_extract(data,'$.key')=?1 AND id<>?2)
             OR EXISTS(SELECT 1 FROM project_key_aliases WHERE key=?1 AND project_id<>?2)",
            params![key, own.map(|id|id.to_string()).unwrap_or_default()], |row|row.get(0),
        )?;
        if !taken { return Ok(key); }
        if explicit.is_some() { return Err(StoreError::Invalid("Projektkürzel ist bereits vergeben")); }
    }
    Err(StoreError::Internal)
}

pub(super) fn reserve_key(tx:&Transaction<'_>,project:ProjectId,key:&str)->Result<()> {
    tx.execute("INSERT OR IGNORE INTO project_key_aliases(key,project_id) VALUES(?1,?2)",
        params![key,project.to_string()])?;
    Ok(())
}

pub(super) fn next_reference(tx: &Transaction<'_>, project: ProjectId) -> Result<String> {
    let prefix: String = tx.query_row("SELECT json_extract(data,'$.key') FROM projects WHERE id=?1",
        [project.to_string()],|row|row.get(0))?;
    tx.execute("INSERT OR IGNORE INTO ticket_counters(project_id) VALUES(?1)",[project.to_string()])?;
    let next: i64 = tx.query_row(
        "UPDATE ticket_counters SET last_number=last_number+1 WHERE project_id=?1 AND last_number<9223372036854775807 RETURNING last_number",
        [project.to_string()],|row|row.get(0),
    )?;
    Ok(format!("{prefix}-{next}"))
}

/// One transaction: stable project order and chronological task order, including
/// drafts and removed projects. Historical events/request receipts stay untouched.
pub(super) fn migrate(tx: &Transaction<'_>) -> Result<()> {
    let projects: Vec<Project> = records(tx,"SELECT data FROM projects ORDER BY rowid")?;
    for project in projects {
        let key = project_key(tx,&project.name,
            (!project.key.is_empty()).then_some(project.key.as_str()),Some(project.id))?;
        tx.execute("UPDATE projects SET data=json_set(data,'$.key',?1) WHERE id=?2",
            params![key,project.id.to_string()])?;
        reserve_key(tx,project.id,&key)?;
        let tickets: Vec<Ticket> = {
            let mut statement=tx.prepare("SELECT data FROM tickets WHERE project_id=?1 ORDER BY json_extract(data,'$.created_unix_ms'),rowid")?;
            let rows=statement.query_map([project.id.to_string()],|row|row.get::<_,String>(0))?;
            rows.map(|raw|Ok(serde_json::from_str(&raw?)?)).collect::<Result<_>>()?
        };
        let mut last=0i64;
        for ticket in tickets.iter().filter(|ticket|!ticket.key.is_empty()) {
            let number=ticket.key.rsplit_once('-').and_then(|(_,number)|number.parse::<i64>().ok())
                .filter(|number|*number>0).ok_or(StoreError::Internal)?;
            last=last.max(number);
        }
        tx.execute("INSERT INTO ticket_counters(project_id,last_number) VALUES(?1,?2)
            ON CONFLICT(project_id) DO UPDATE SET last_number=MAX(last_number,excluded.last_number)",
            params![project.id.to_string(),last])?;
        for ticket in tickets.into_iter().filter(|ticket|ticket.key.is_empty()) {
            let reference=next_reference(tx,project.id)?;
            tx.execute("UPDATE tickets SET data=json_set(data,'$.key',?1) WHERE id=?2",
                params![reference,ticket.id.to_string()])?;
        }
    }
    Ok(())
}

/// Renaming the prefix preserves sequence numbers and never resets the counter.
pub(super) fn rename(tx: &Transaction<'_>, project: ProjectId, key: &str) -> Result<()> {
    reserve_key(tx,project,key)?;
    let mut statement=tx.prepare("SELECT data FROM tickets WHERE project_id=?1 ORDER BY rowid")?;
    let rows=statement.query_map([project.to_string()],|row|row.get::<_,String>(0))?;
    let tickets:Vec<Ticket>=rows.map(|raw|Ok(serde_json::from_str(&raw?)?)).collect::<Result<_>>()?;
    for mut ticket in tickets {
        let suffix=ticket.key.rsplit_once('-').ok_or(StoreError::Internal)?.1;
        ticket.key=format!("{key}-{suffix}");
        ticket.revision.0=ticket.revision.0.checked_add(1).ok_or(StoreError::Internal)?;
        tx.execute("UPDATE tickets SET revision=?1,data=?2 WHERE id=?3",
            params![ticket.revision.0,serde_json::to_string(&ticket)?,ticket.id.to_string()])?;
        event(tx,"ticket.updated",ticket.id.to_string(),&ticket)?;
    }
    Ok(())
}
