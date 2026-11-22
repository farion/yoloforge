use super::*;
impl Store {
    pub fn set_session_limit(&self, id: ProjectId, b: SetSessionLimit) -> Result<Project> {
        self.mutate(
            b.request_id,
            &format!("project.session_limit/{id}"),
            &b,
            |tx| {
                if !(1..=16).contains(&b.max_sessions) {
                    return Err(StoreError::Invalid(
                        "Sessionlimit muss zwischen 1 und 16 liegen",
                    ));
                }
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
                p.max_sessions = b.max_sessions;
                p.revision.0 += 1;
                tx.execute(
                    "UPDATE projects SET data=?1 WHERE id=?2",
                    params![serde_json::to_string(&p)?, id.to_string()],
                )?;
                super::scheduler::set_limit(tx, id, b.max_sessions)?;
                event(tx, "project.session_limit", id.to_string(), &p)?;
                Ok(p)
            },
        )
    }
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
            if let Some(root)=b.requested_session {
                let data:Option<String>=tx.query_row("SELECT c.data FROM commands c JOIN streams s ON s.id=c.stream_id WHERE c.id=?1 AND s.project_id=?2",params![root.to_string(),b.project_id.to_string()],|r|r.get(0)).optional()?;
                let c:RunnerCommand=serde_json::from_str(&data.ok_or(StoreError::Invalid("Session gehoert nicht zu diesem Projekt"))?)?;
                if c.kind!=CommandKind::RunAgent || c.session_root()!=root || c.agent_spec.as_ref().is_none_or(|a|a.read_only){return Err(StoreError::Invalid("Keine auswaehlbare Arbeitssession"));}
            }
            let mut t=if let Some(id)=b.id {
                let data:Option<String>=tx.query_row("SELECT data FROM tickets WHERE id=?1",[id.to_string()],|r|r.get(0)).optional()?;let t:Ticket=serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if t.project_id!=b.project_id{return Err(StoreError::WorkConflict);}
                if t.revision!=b.expected_revision{return Err(StoreError::RevisionConflict(t.revision));}
                let assigned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM stream_tickets WHERE ticket_id=?1)",[id.to_string()],|r|r.get(0))?;
                if assigned{return Err(StoreError::Invalid("Aufgabe bereits eingeplant; neuen Prompt als Folgeaufgabe speichern"));}t
            }else{if b.expected_revision!=Revision(0){return Err(StoreError::RevisionConflict(Revision(0)));}Ticket{execution_command:None,requested_session:None,routing_note:String::new(),id:TicketId::new(),project_id:b.project_id,revision:Revision(0),title:String::new(),description:String::new(),acceptance_criteria:String::new(),priority:1,status:TicketState::Backlog,created_unix_ms:now,updated_unix_ms:now,depends_on:vec![],prompt:None,images:vec![]}};
            t.title=b.prompt.lines().next().unwrap_or("").chars().take(80).collect();if t.title.trim().is_empty(){t.title="Bildaufgabe".into();}
            t.description=b.prompt.clone();t.prompt=Some(b.prompt.clone());t.images=b.images.clone();t.revision.0+=1;t.updated_unix_ms=now;
            t.requested_session=b.requested_session;t.routing_note=if b.requested_session.is_some(){"Fest an ausgewaehlte Session gebunden".into()}else{"Wartet auf Orchestrator".into()};
            tx.execute("INSERT INTO tickets(id,project_id,revision,data) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data",params![t.id.to_string(),t.project_id.to_string(),t.revision.0,serde_json::to_string(&t)?])?;
            super::scheduler::activate_prompts(tx,t.project_id)?;
            event(tx,"task.saved",t.id.to_string(),&t)?;Ok(t)
        })
    }
}
