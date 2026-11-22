use super::*;
impl Store {
    pub fn pause_project(&self, id: ProjectId, b: PauseProject) -> Result<Project> {
        self.mutate(b.request_id, &format!("project.pause/{id}"), &b, |tx| {
            let data: Option<String> = tx
                .query_row(
                    "SELECT data FROM projects WHERE id=?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            let mut p: Project = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
            if p.revision != b.expected_revision {
                return Err(StoreError::RevisionConflict(p.revision));
            }
            p.paused = b.paused;
            p.revision.0 += 1;
            tx.execute(
                "UPDATE projects SET data=?1 WHERE id=?2",
                params![serde_json::to_string(&p)?, id.to_string()],
            )?;
            event(tx, "project.paused", id.to_string(), &p)?;
            Ok(p)
        })
    }
    pub fn save_task(&self, b: SaveTask) -> Result<Ticket> {
        self.mutate(b.request_id,"task.save",&b,|tx|{
            if (b.prompt.trim().is_empty() && b.images.is_empty()) || b.prompt.len()>12000 || b.prompt.chars().any(|c|c.is_control() && !matches!(c,'\n'|'\r'|'\t')){return Err(StoreError::Invalid("Aufgabe braucht Text oder Bilder; Text maximal 12000 UTF-8 Bytes"));}
            validate_images(&b.images).map_err(StoreError::Invalid)?;
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",[b.project_id.to_string()],|r|r.get(0))?;if !exists{return Err(StoreError::NotFound);}
            let now=super::super::unix_ms();
            let mut t=if let Some(id)=b.id {
                let data:Option<String>=tx.query_row("SELECT data FROM tickets WHERE id=?1",[id.to_string()],|r|r.get(0)).optional()?;let t:Ticket=serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if t.project_id!=b.project_id{return Err(StoreError::WorkConflict);}
                if t.revision!=b.expected_revision{return Err(StoreError::RevisionConflict(t.revision));}
                let assigned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM stream_tickets WHERE ticket_id=?1)",[id.to_string()],|r|r.get(0))?;
                if assigned{return Err(StoreError::Invalid("Aufgabe bereits eingeplant; neuen Prompt als Folgeaufgabe speichern"));}t
            }else{if b.expected_revision!=Revision(0){return Err(StoreError::RevisionConflict(Revision(0)));}Ticket{id:TicketId::new(),project_id:b.project_id,revision:Revision(0),title:String::new(),description:String::new(),acceptance_criteria:String::new(),priority:1,status:TicketState::Backlog,created_unix_ms:now,updated_unix_ms:now,depends_on:vec![],prompt:None,images:vec![]}};
            t.title=b.prompt.lines().next().unwrap_or("").chars().take(80).collect();if t.title.trim().is_empty(){t.title="Bildaufgabe".into();}
            t.description=b.prompt.clone();t.prompt=Some(b.prompt.clone());t.images=b.images.clone();t.revision.0+=1;t.updated_unix_ms=now;
            tx.execute("INSERT INTO tickets(id,project_id,revision,data) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data",params![t.id.to_string(),t.project_id.to_string(),t.revision.0,serde_json::to_string(&t)?])?;
            super::scheduler::activate_prompts(tx,t.project_id)?;
            event(tx,"task.saved",t.id.to_string(),&t)?;Ok(t)
        })
    }
}
