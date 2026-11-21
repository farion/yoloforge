use super::*;

fn scope(tx: &Transaction<'_>, project: ProjectId, ticket: Option<TicketId>) -> Result<()> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
        [project.to_string()],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(StoreError::NotFound);
    }
    if let Some(ticket) = ticket {
        let valid: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tickets WHERE id=?1 AND project_id=?2)",
            params![ticket.to_string(), project.to_string()],
            |r| r.get(0),
        )?;
        if !valid {
            return Err(StoreError::Invalid(
                "Knowledge ticket belongs to another project or is missing",
            ));
        }
    }
    Ok(())
}

impl Store {
    pub fn create_knowledge(&self, body: CreateKnowledge) -> Result<KnowledgeNote> {
        self.mutate(body.request_id, "knowledge.create", &body, |tx| {
            Self::create_knowledge_in(tx, &body)
        })
    }
    pub(super) fn create_knowledge_in(
        tx: &Transaction<'_>,
        body: &CreateKnowledge,
    ) -> Result<KnowledgeNote> {
        scope(tx, body.project_id, body.ticket_id)?;
        line(&body.title, 160)?;
        line(&body.source, 512)?;
        if body.content.trim().is_empty()
            || body.content.len() > 4096
            || body
                .content
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            || body.priority > 9
        {
            return Err(StoreError::Invalid("Invalid knowledge content or priority"));
        }
        if let Some(command) = body.source_command {
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands c JOIN streams s ON s.id=c.stream_id WHERE c.id=?1 AND s.project_id=?2)",params![command.to_string(),body.project_id.to_string()],|r|r.get(0))?;
            if !valid {
                return Err(StoreError::Invalid(
                    "Knowledge source command belongs to another project or is missing",
                ));
            }
        }
        let note = KnowledgeNote {
            id: KnowledgeId::new(),
            project_id: body.project_id,
            ticket_id: body.ticket_id,
            revision: Revision(1),
            title: body.title.clone(),
            content: body.content.clone(),
            source: body.source.clone(),
            source_command: body.source_command,
            priority: body.priority,
            retired: false,
            created_unix_ms: super::super::unix_ms(),
        };
        tx.execute(
            "INSERT INTO knowledge(id,project_id,ticket_id,priority,data) VALUES(?1,?2,?3,?4,?5)",
            params![
                note.id.to_string(),
                note.project_id.to_string(),
                note.ticket_id.map(|id| id.to_string()),
                note.priority,
                serde_json::to_string(&note)?
            ],
        )?;
        event(tx, "knowledge.created", note.id.to_string(), &note)?;
        Ok(note)
    }
    pub fn retire_knowledge(
        &self,
        id: KnowledgeId,
        body: RetireKnowledge,
    ) -> Result<KnowledgeNote> {
        self.mutate(
            body.request_id,
            &format!("knowledge.retire/{id}"),
            &body,
            |tx| {
                let data: Option<String> = tx
                    .query_row(
                        "SELECT data FROM knowledge WHERE id=?1",
                        [id.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?;
                let mut note: KnowledgeNote =
                    serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if note.revision != body.expected_revision {
                    return Err(StoreError::RevisionConflict(note.revision));
                }
                if !note.retired {
                    note.retired = true;
                    note.revision.0 += 1;
                    tx.execute(
                        "UPDATE knowledge SET retired=1,data=?1 WHERE id=?2",
                        params![serde_json::to_string(&note)?, id.to_string()],
                    )?;
                    event(tx, "knowledge.retired", id.to_string(), &note)?;
                }
                Ok(note)
            },
        )
    }
    /// Bounded newest-first view, including retired provenance. Full history is in events.
    pub fn knowledge(&self, project: ProjectId) -> Result<Vec<KnowledgeNote>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        scope(&tx, project, None)?;
        let mut stmt = tx.prepare(
            "SELECT data FROM knowledge WHERE project_id=?1 ORDER BY rowid DESC LIMIT 200",
        )?;
        let rows = stmt.query_map([project.to_string()], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn knowledge_context(
        &self,
        project: ProjectId,
        ticket: Option<TicketId>,
        budget: usize,
    ) -> Result<KnowledgeContext> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        scope(&tx, project, ticket)?;
        Self::select_knowledge(
            &tx,
            project,
            &ticket.into_iter().collect::<Vec<_>>(),
            budget,
        )
    }
    pub(super) fn select_knowledge(
        tx: &Transaction<'_>,
        project: ProjectId,
        tickets: &[TicketId],
        budget: usize,
    ) -> Result<KnowledgeContext> {
        if budget > 16384 {
            return Err(StoreError::Invalid("Knowledge budget exceeds 16384 bytes"));
        }
        // Filter scope in SQL before limiting; unrelated ticket notes cannot crowd out project notes.
        let ticket_json = serde_json::to_string(tickets)?;
        let filter="project_id=?1 AND retired=0 AND (ticket_id IS NULL OR ticket_id IN (SELECT value FROM json_each(?2)))";
        let total: usize = tx.query_row(
            &format!("SELECT count(*) FROM knowledge WHERE {filter}"),
            params![project.to_string(), ticket_json],
            |r| r.get(0),
        )?;
        let mut stmt = tx.prepare(&format!(
            "SELECT data FROM knowledge WHERE {filter} ORDER BY priority DESC,rowid DESC LIMIT 256"
        ))?;
        let rows = stmt.query_map(params![project.to_string(), ticket_json], |r| {
            r.get::<_, String>(0)
        })?;
        let mut context = KnowledgeContext {
            notes: Vec::new(),
            text: String::new(),
            budget_bytes: budget,
            omitted: total,
        };
        for row in rows {
            let note: KnowledgeNote = serde_json::from_str(&row?)?;
            let item = format!(
                "\n[Knowledge {} revision {}]\n{}\nSource: {}\n{}\n",
                note.id, note.revision.0, note.title, note.source, note.content
            );
            if context.text.len() + item.len() <= budget {
                context.text.push_str(&item);
                context.notes.push(note);
                context.omitted -= 1;
            }
        }
        Ok(context)
    }
}
