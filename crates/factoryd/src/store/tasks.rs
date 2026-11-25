use super::*;
use factory_protocol::models::{ModelSelection, SubmitTaskWithModel};
pub(super) fn editable(db: &Connection, t: &Ticket) -> Result<bool> {
    if t.execution_command.is_some()
        || !matches!(t.status, TicketState::Backlog | TicketState::Ready)
    {
        return Ok(false);
    }
    Ok(!db.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM active_projects WHERE id=?2)
        OR EXISTS(SELECT 1 FROM stream_tickets WHERE ticket_id=?1)
        OR EXISTS(SELECT 1 FROM scheduler_runs WHERE ticket_id=?1)
        OR EXISTS(SELECT 1 FROM task_routing WHERE ticket_id=?1)
        OR EXISTS(SELECT 1 FROM events WHERE aggregate_id=?1 AND kind='task.routing')",
        params![t.id.to_string(), t.project_id.to_string()],
        |r| r.get::<_, bool>(0),
    )?)
}
impl Store {
    pub fn begin_task_edit(&self, id: TicketId, body: BeginTaskEdit) -> Result<TaskEdit> {
        self.mutate(body.request_id,&format!("task.edit/{id}"),&body,|tx| {
            let raw:Option<String>=tx.query_row("SELECT data FROM tickets WHERE id=?1",[id.to_string()],|r|r.get(0)).optional()?;
            let mut ticket:Ticket=serde_json::from_str(&raw.ok_or(StoreError::NotFound)?)?;
            if ticket.revision!=body.expected_revision {return Err(StoreError::RevisionConflict(ticket.revision));}
            if !editable(tx,&ticket)? {return Err(StoreError::Invalid("Die Factory hat diese Aufgabe bereits übernommen. Bearbeiten ist nicht mehr möglich"));}
            let held:Option<String>=tx.query_row("SELECT token FROM task_edits WHERE ticket_id=?1",[id.to_string()],|r|r.get(0)).optional()?;
            let token=if let Some(held)=held {
                if body.edit_token.map(|id|id.to_string()).as_deref()!=Some(&held) {return Err(StoreError::Invalid("Diese Aufgabe wird bereits in einem anderen Entwurf bearbeitet"));}
                body.edit_token.ok_or(StoreError::Internal)?
            } else {
                let token=body.edit_token.unwrap_or(body.request_id);
                tx.execute("INSERT INTO task_edits(ticket_id,token) VALUES(?1,?2)",params![id.to_string(),token.to_string()])?;
                ticket.draft=true;
                ticket.revision.0=ticket.revision.0.checked_add(1).ok_or(StoreError::Internal)?;
                ticket.updated_unix_ms=super::super::unix_ms();
                ticket.routing_note="In Bearbeitung als Entwurf — nicht zur Ausführung freigegeben".into();
                tx.execute("UPDATE tickets SET revision=?1,data=?2 WHERE id=?3",params![ticket.revision.0,serde_json::to_string(&ticket)?,id.to_string()])?;
                event(tx,"task.edit_started",id.to_string(),&ticket)?;
                token
            };
            let local_mcps = super::mcp::task_optional(tx, ticket.id)?;
            Ok(TaskEdit {ticket,edit_token:token, local_mcps})
        })
    }
    pub fn delete_project(&self, id: ProjectId, body: DeleteProject) -> Result<Project> {
        self.mutate(body.request_id,&format!("project.delete/{id}"),&body,|tx| {
            let raw:Option<String>=tx.query_row("SELECT data FROM active_projects WHERE id=?1",[id.to_string()],|row|row.get(0)).optional()?;
            let mut project:Project=serde_json::from_str(&raw.ok_or(StoreError::NotFound)?)?;
            if project.revision!=body.expected_revision {return Err(StoreError::RevisionConflict(project.revision));}
            if body.confirm_name!=project.name {return Err(StoreError::Invalid("Zum Löschen den vollständigen Projektnamen bestätigen"));}
            let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands c JOIN streams s ON s.id=c.stream_id WHERE s.project_id=?1 AND c.state IN ('pending','accepted'))",[id.to_string()],|row|row.get(0))?;
            if busy {return Err(StoreError::Invalid("Projekt hat noch laufende oder eingereihte Aufträge und kann nicht gelöscht werden"));}
            project.deleted=true;
            project.paused=true;
            project.revision.0=project.revision.0.checked_add(1).ok_or(StoreError::Internal)?;
            tx.execute("UPDATE projects SET data=?1 WHERE id=?2",params![serde_json::to_string(&project)?,id.to_string()])?;
            for table in ["scheduler_policies","supervisor_policies"] {
                tx.execute(&format!("UPDATE {table} SET data=json_set(data,'$.enabled',json('false'),'$.revision',json_extract(data,'$.revision')+1) WHERE project_id=?1"),[id.to_string()])?;
            }
            event(tx,"project.deleted",id.to_string(),&project)?;
            Ok(project)
        })
    }
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
                        "SELECT data FROM active_projects WHERE id=?1",
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
                    "SELECT data FROM active_projects WHERE id=?1",
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
        self.submit_task_in(
            SubmitTask {
                edit_token: None,
                task: b,
                draft: false,
                files: vec![],
            },
            true,
            None,
            None,
            false,
        )
    }
    pub fn submit_task(&self, submission: SubmitTask) -> Result<Ticket> {
        self.submit_task_in(submission, false, None, None, false)
    }
    pub fn submit_task_with_model(&self, body: SubmitTaskWithModel) -> Result<Ticket> {
        self.submit_task_in(body.submission, false, Some(&body.implementer), body.local_mcps.as_deref(), false)
    }
    pub fn prompt_session(&self, root: CommandId, body: PromptSession) -> Result<Ticket> {
        let project = {
            let db = self.0.lock().map_err(|_| StoreError::Internal)?;
            db.query_row("SELECT s.project_id FROM commands c JOIN streams s ON s.id=c.stream_id WHERE c.id=?1",
                [root.to_string()], |r| r.get::<_, String>(0)).optional()?.ok_or(StoreError::NotFound)?
                .parse().map_err(|_| StoreError::Internal)?
        };
        self.submit_task_in(SubmitTask {
            task: SaveTask {
                request_id: body.request_id, id: None, expected_revision: Revision(0),
                project_id: project, requested_session: Some(root), prompt: body.prompt, images: body.images,
            },
            files: body.files, draft: false, edit_token: None,
        }, false, None, None, true)
    }
    fn submit_task_in(&self, submission: SubmitTask, legacy: bool, implementer: Option<&Option<ModelSelection>>, requested_mcps: Option<&[String]>, direct: bool) -> Result<Ticket> {
        let b = &submission.task;
        // Preserve the legacy fingerprint so a previously accepted SaveTask is replayable.
        #[derive(serde::Serialize)]
        #[serde(untagged)]
        enum Fingerprint<'a> {
            Legacy(&'a SaveTask),
            Extended(&'a SubmitTask),
            SessionPrompt(&'a SubmitTask),
            WithModel { submission: &'a SubmitTask, implementer: &'a Option<ModelSelection>,
                #[serde(skip_serializing_if = "Option::is_none")] local_mcps: Option<&'a [String]> },
        }
        let fingerprint = if direct {
            Fingerprint::SessionPrompt(&submission)
        } else if legacy {
            Fingerprint::Legacy(b)
        } else if let Some(implementer) = implementer {
            Fingerprint::WithModel { submission: &submission, implementer, local_mcps: requested_mcps }
        } else {
            Fingerprint::Extended(&submission)
        };
         let result = self.mutate(b.request_id,if direct { "session.prompt" } else if legacy { "task.save" } else if implementer.is_some() { "task.submit_with_model" } else { "task.submit" },&fingerprint,|tx|{
            if b.prompt.trim().is_empty() && b.images.is_empty() && submission.files.is_empty() {
                return Err(StoreError::Invalid("Intent braucht Text oder Anhänge"));
            }
            if b.prompt.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
                return Err(StoreError::Invalid("Prompt enthält ungültige Steuerzeichen"));
            }
            validate_images(&b.images).map_err(StoreError::Invalid)?;
            validate_files(&submission.files).map_err(StoreError::Invalid)?;
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM active_projects WHERE id=?1)",[b.project_id.to_string()],|r|r.get(0))?;if !exists{return Err(StoreError::NotFound);}
            let now=super::super::unix_ms();
             let session = if let Some(root)=b.requested_session {
                let data:Option<String>=tx.query_row("SELECT c.data FROM commands c JOIN streams s ON s.id=c.stream_id WHERE c.id=?1 AND s.project_id=?2",params![root.to_string(),b.project_id.to_string()],|r|r.get(0)).optional()?;
                let c:RunnerCommand=serde_json::from_str(&data.ok_or(StoreError::Invalid("Session gehoert nicht zu diesem Projekt"))?)?;
                 if c.kind!=CommandKind::RunAgent || c.session_root()!=root || c.agent_spec.as_ref().is_none_or(|a|a.read_only){return Err(StoreError::Invalid("Keine auswaehlbare Arbeitssession"));}
                 if direct && tx.query_row(
                     "SELECT EXISTS(SELECT 1 FROM runtime_terminations WHERE stream_id=?1)",
                     [c.stream_id.to_string()], |r| r.get::<_, bool>(0))? {
                     return Err(StoreError::Invalid("Session-Laufzeit wurde beendet oder wird beendet"));
                 }
                 c.agent_spec
             } else { None };
            let mut t=if let Some(id)=b.id {
                let data:Option<String>=tx.query_row("SELECT data FROM tickets WHERE id=?1",[id.to_string()],|r|r.get(0)).optional()?;let t:Ticket=serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if t.project_id!=b.project_id{return Err(StoreError::WorkConflict);}
                if legacy && !t.files.is_empty() { return Err(StoreError::Invalid("Dieser Client kann Datei-Anhänge nicht bearbeiten; Desktop-Aufgabendialog verwenden")); }
                if t.revision!=b.expected_revision{return Err(StoreError::RevisionConflict(t.revision));}
                if !editable(tx,&t)? {return Err(StoreError::Invalid("Aufgabe bereits verarbeitet; neuen Prompt als Folgeaufgabe speichern"));}
                let held:Option<String>=tx.query_row("SELECT token FROM task_edits WHERE ticket_id=?1",[id.to_string()],|r|r.get(0)).optional()?;
                if held.is_some() && held!=submission.edit_token.map(|id|id.to_string()) {return Err(StoreError::Invalid("Bearbeitung gehört zu einem anderen Entwurf"));}
                t
            }else{if b.expected_revision!=Revision(0){return Err(StoreError::RevisionConflict(Revision(0)));}Ticket{key:super::numbering::next_reference(tx,b.project_id)?,draft:false,files:vec![],execution_command:None,requested_session:None,routing_note:String::new(),id:TicketId::new(),project_id:b.project_id,revision:Revision(0),title:String::new(),description:String::new(),acceptance_criteria:String::new(),priority:1,status:TicketState::Backlog,created_unix_ms:now,updated_unix_ms:now,depends_on:vec![],prompt:None,images:vec![]}};
            t.draft = submission.draft;
            t.files = submission.files.clone();
            t.title=b.prompt.lines().next().unwrap_or("").chars().take(80).collect();if t.title.trim().is_empty(){t.title=if !submission.files.is_empty() { "Dateiaufgabe" } else { "Bildaufgabe" }.into();}
            t.description=b.prompt.clone();t.prompt=Some(b.prompt.clone());t.images=b.images.clone();t.revision.0+=1;t.updated_unix_ms=now;
             t.requested_session=b.requested_session;t.routing_note=if t.draft { "Entwurf — noch nicht freigegeben".into() } else if b.requested_session.is_some(){"Fest an ausgewaehlte Session gebunden".into()}else{"Wartet auf Orchestrator".into()};
               let models = if direct {
                   let agent = session.as_ref().ok_or(StoreError::Invalid("Arbeitssession fehlt"))?;
                   if super::models::factory(tx)?.is_some() {
                       let model = ModelSelection { access_id: agent.profile.provider_id.clone(), model_id: agent.profile.model_id.clone() };
                       super::models::require_available(tx, &model)?;
                       Some(factory_protocol::models::TaskModels {
                           implementer_override: Some(model.clone()), implementer: Some(model),
                           orchestrator: None,
                           supervisor: super::models::role_selection(tx, t.project_id, "supervisor")?,
                           memorizer: super::models::role_selection(tx, t.project_id, "memorizer")?,
                       })
                   } else { None }
               } else { super::models::for_task(tx, t.id, t.project_id, implementer, t.draft)? };
               let selected_skills = super::skills::selected(tx, t.project_id)?;
               let repo_skills = if direct {
                   selected_skills.for_session_prompt(session.as_ref().ok_or(StoreError::Internal)?)
                       .ok_or(StoreError::Invalid("Mindestens ein Skill dieser Session ist nicht mehr freigegeben; neue Session wählen"))?
               } else { selected_skills };
                let optional_mcps = match requested_mcps {
                    Some(requested) => requested.to_vec(),
                    None if b.id.is_none() && !direct => super::mcp::default_optional(tx, t.project_id)?,
                    None => super::mcp::task_optional(tx, t.id)?,
               };
                let local_mcps = if direct {
                    let agent = session.as_ref().ok_or(StoreError::Invalid("Arbeitssession fehlt"))?;
                    let pinned = McpProfile { local: agent.local_mcps.clone(), remote: agent.remote_mcps.clone() };
                    if !super::mcp::allowed(tx, t.project_id, &pinned)? {
                        return Err(StoreError::Invalid("MCP-Freigabe der Session ist nicht mehr gültig"));
                    }
                    pinned
                } else { super::mcp::resolve(tx, t.project_id, &optional_mcps)? };
                if !t.draft && session.as_ref().is_some_and(|agent|
                    (!direct && !repo_skills.matches(agent)) || !local_mcps.matches(agent)) {
                  return Err(StoreError::Invalid("Skill-Freigabe oder MCP-Auswahl passt nicht zur ausgewählten Session; neue Session wählen"));
              }
             if let Some((model, agent)) = models.as_ref().and_then(|m| m.implementer.as_ref()).zip(session.as_ref()) {
                 if agent.profile.provider_id != model.access_id || agent.profile.model_id != model.model_id {
                     return Err(StoreError::Invalid("Gewähltes Modell oder Zugang passt nicht zur ausgewählten Session; neue Session wählen"));
                 }
             }
              tx.execute("INSERT INTO tickets(id,project_id,revision,data) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data",params![t.id.to_string(),t.project_id.to_string(),t.revision.0,serde_json::to_string(&t)?])?;
              if !t.draft {
                  super::skills::pin(tx, t.id, &repo_skills)?;
               }
               super::mcp::pin(tx, t.id, local_mcps, optional_mcps)?;
             if let Some(models) = models {
                 super::models::save_ticket(tx, t.id, &models)?;
             }
            tx.execute("DELETE FROM task_edits WHERE ticket_id=?1",[t.id.to_string()])?;
            if !t.draft && legacy { super::scheduler::activate_prompts(tx,t.project_id)?; }
             if direct {
                 super::sessions::reopen_for_prompt(tx, b.requested_session.ok_or(StoreError::Internal)?, t.id)?;
             }
             event(tx,"task.saved",t.id.to_string(),&t)?;Ok(t)
        });
        if result.is_ok() && !submission.draft {
            self.wake_scheduler();
        }
        result
    }
}
