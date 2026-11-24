use super::*;

impl Store {
    pub fn task_history(&self, id: TicketId, before: Option<u64>) -> Result<TaskHistoryPage> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let id = id.to_string();
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM tickets WHERE id=?1)", [&id], |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound);
        }
        // Consult the assignment journal, not all commands on a reused session.
        let mut statement = db.prepare(
            "WITH assigned_commands(id) AS (
                SELECT r.prepare_id FROM scheduler_runs r WHERE r.ticket_id=?1
                  AND r.rowid=(SELECT MIN(first.rowid) FROM scheduler_runs first WHERE first.prepare_id=r.prepare_id)
                UNION SELECT agent_id FROM scheduler_runs WHERE ticket_id=?1 AND agent_id IS NOT NULL
                UNION SELECT json_extract(data,'$.execution_command') FROM tickets WHERE id=?1
             ), related(id) AS (
                SELECT id FROM assigned_commands WHERE id IS NOT NULL
                UNION SELECT id FROM questions WHERE command_id IN (SELECT id FROM assigned_commands)
                UNION SELECT ?1
             )
             SELECT sequence, kind, created_unix_ms, payload FROM events
             WHERE aggregate_id IN (SELECT id FROM related) AND sequence<?2
               AND kind IN ('ticket.created','ticket.updated','task.saved','task.edit_started',
                   'task.routing','scheduler.blocked','command.created','command.updated',
                   'question.created','question.updated')
             ORDER BY sequence DESC LIMIT 101",
        )?;
        let rows = statement.query_map(
            params![id, before.unwrap_or(i64::MAX as u64).min(i64::MAX as u64)],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?,
                      row.get::<_, u64>(2)?, row.get::<_, String>(3)?)),
        )?.collect::<std::result::Result<Vec<_>, _>>()?;
        let next_before = (rows.len() > 100).then(|| rows[99].0);
        // A command can publish multiple updates in one state. Collapse adjacent
        // repetitions on the same aggregate while preserving distinct questions.
        let mut entries = Vec::new();
        let mut previous = None;
        for (sequence, kind, created_unix_ms, payload) in rows.into_iter().take(100) {
            let payload: serde_json::Value = serde_json::from_str(&payload)?;
            if let Some((message, level)) = summarize(&kind, &payload) {
                let dedup = payload["id"].as_str().map(|id| (id.to_owned(), message.clone(), level));
                if dedup.is_some() && dedup == previous && kind == "command.updated" {
                    continue;
                }
                previous = dedup;
                entries.push(TaskHistoryEntry { sequence, created_unix_ms, message, level });
            }
        }
        Ok(TaskHistoryPage { entries, next_before })
    }
}

fn summarize(kind: &str, data: &serde_json::Value) -> Option<(String, TaskHistoryLevel)> {
    use TaskHistoryLevel::*;
    let line = |text: &str, level| Some((text.to_owned(), level));
    match kind {
        "ticket.created" => line("Aufgabe angelegt", Info),
        "task.saved" if data["draft"] == true => line("Entwurf gespeichert", Info),
        "task.saved" => line("Aufgabe eingereiht", Info),
        "task.edit_started" => line("Aufgabe zur Bearbeitung geöffnet", Info),
        "task.routing" => {
            let note = data["routing_note"].as_str()?;
            line(note, task_routing_level(note))
        }
        "scheduler.blocked" if data["missing_model"] != true => {
            line(data["reason"].as_str().unwrap_or("Aufgabe blockiert"), Error)
        }
        "ticket.updated" => match data["status"].as_str()? {
            "done" => line("Aufgabe abgeschlossen", Success),
            "review" => line("Ergebnis liegt vor", Success),
            "blocked" => line("Aufgabe blockiert", Error),
            "cancelled" => line("Aufgabe abgebrochen", Warning),
            _ => line("Aufgabe aktualisiert", Info),
        },
        "question.created" => line("Rückfrage eingegangen", Warning),
        "question.updated" => match data["state"].as_str()? {
            "answered" => line("Rückfrage beantwortet", Success),
            "answer_queued" => line("Antwort eingereiht", Info),
            "rejected" | "unknown" => line("Antwort nicht bestätigt", Error),
            _ => None,
        },
        "command.created" | "command.updated" => {
            match (data["kind"].as_str()?, data["state"].as_str()?) {
                ("prepare_workspace", "pending") => line("Arbeitsbereich wird vorbereitet", Info),
                ("prepare_workspace", "succeeded") => line("Arbeitsbereich vorbereitet", Success),
                ("prepare_workspace", "failed") => {
                    let reason = data["result"]["workspace_error"].as_str().unwrap_or("Unbekannter Fehler");
                    line(&format!("Arbeitsbereich konnte nicht vorbereitet werden: {reason}"), Error)
                }
                ("run_agent", "accepted") if data["agent_progress"]["phase"] == "running" =>
                    line("Bearbeitung läuft", Info),
                ("run_agent", "pending") => line("Bearbeitung an Runner übergeben", Info),
                ("run_agent", "succeeded") => line("Bearbeitung abgeschlossen", Success),
                ("run_agent", "failed") => line("Bearbeitung fehlgeschlagen", Error),
                _ => None,
            }
        }
        _ => None,
    }
}
