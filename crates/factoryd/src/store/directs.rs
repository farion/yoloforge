use super::*;
use factory_protocol::{directs::*, models::ModelSelection};

#[derive(serde::Serialize, serde::Deserialize)]
struct Turn {
    prompt: String,
    images: Vec<PromptImage>,
    files: Vec<PromptFile>,
    mcps: Option<McpProfile>,
}

impl Store {
    pub fn list_directs(&self, project: ProjectId) -> Result<Vec<Direct>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        list(&db, Some(project))
    }

    pub fn create_direct(&self, body: CreateDirect) -> Result<Direct> {
        self.mutate(body.request_id, "direct.create", &body, |tx| {
            project(tx, body.project_id)?;
            let direct = Direct {
                id: CommandId::new(),
                project_id: body.project_id,
                state: DirectState::Open,
                created_unix_ms: super::super::unix_ms(),
                stream_id: None,
                selection: None,
                root_command_id: None,
                current_command_id: None,
                preparation_failed: false,
            };
            tx.execute(
                "INSERT INTO directs(id,project_id,data) VALUES(?1,?2,?3)",
                params![direct.id.to_string(), direct.project_id.to_string(), serde_json::to_string(&direct)?],
            )?;
            event(tx, "direct.created", direct.id.to_string(), &direct)?;
            Ok(direct)
        })
    }

    pub fn prompt_direct(&self, id: CommandId, body: PromptDirect) -> Result<Direct> {
        self.prompt_direct_with_runner_support(id, body, true)
    }

    pub(crate) fn prompt_direct_with_runner_support(&self, id: CommandId, body: PromptDirect, runner_supports_directs: bool) -> Result<Direct> {
        self.mutate(body.request_id, &format!("direct.prompt/{id}"), &body, |tx| {
            let mut direct = load(tx, id)?;
            if direct.state != DirectState::Open || busy(tx, &direct)? {
                return Err(StoreError::WorkConflict);
            }
            validate_turn(&body)?;
            let project = project(tx, direct.project_id)?;
            if project.paused {
                return Err(StoreError::WorkConflict);
            }
            if direct.selection.is_some() != body.selection.is_none() {
                return Err(StoreError::Invalid("Direct settings can only be set on the first prompt"));
            }
            let mut pinned_mcps = None;
            if let Some(selection) = &body.selection {
                if !runner_supports_directs {
                    return Err(StoreError::Invalid("Runner does not support Directs"));
                }
                if !project.runner_bindings.contains_key(&selection.runner_id) {
                    return Err(StoreError::Invalid("Runner is not bound to the project"));
                }
                let model = match &selection.model {
                    Some(model) => Some(model.clone()),
                    None => super::models::role_selection(tx, project.id, "implementer")?,
                };
                if let Some(model) = &model {
                    super::models::require_available(tx, model)?;
                }
                let model = model.unwrap_or_else(|| ModelSelection {
                    access_id: project.profile.provider_id.clone(),
                    model_id: project.profile.model_id.clone(),
                });
                line(&model.access_id, 120)?;
                line(&model.model_id, 200)?;
                let optional = match &selection.optional_mcps {
                    Some(mcps) => mcps.clone(),
                    None => super::mcp::default_optional(tx, project.id)?,
                };
                let mcps = super::mcp::resolve(tx, project.id, &optional)?;
                if !super::mcp::runner_ready(tx, selection.runner_id, &mcps)? {
                    return Err(StoreError::Invalid("Runner cannot use the selected MCPs"));
                }
                pinned_mcps = Some(mcps);
                let owner = Store::create_stream_in(tx, &CreateStream {
                    request_id: body.request_id,
                    project_id: project.id,
                    goal: "Direct".into(),
                    runner_id: selection.runner_id,
                    ticket_ids: vec![],
                })?;
                direct.stream_id = Some(owner.id);
                direct.selection = Some(DirectSelection {
                    runner_id: selection.runner_id,
                    model: Some(model),
                    optional_mcps: Some(optional),
                });
            }
            if direct.preparation_failed {
                let old: Option<String> = tx.query_row("SELECT pending_turn FROM directs WHERE id=?1",
                    [id.to_string()], |row| row.get(0))?;
                if let Some(old) = old {
                    pinned_mcps = serde_json::from_str::<Turn>(&old)?.mcps;
                }
            }
            let turn = Turn { prompt: body.prompt.clone(), images: body.images.clone(), files: body.files.clone(), mcps: pinned_mcps };
            let stream_id = direct.stream_id.ok_or(StoreError::Internal)?;
            if direct.root_command_id.is_none() {
                let owner = super::work::stream(tx, stream_id)?;
                let prepare = Store::enqueue_role(tx, stream_id, &EnqueueCommand {
                    request_id: body.request_id, expected_revision: owner.revision,
                    kind: CommandKind::PrepareWorkspace, prompt: None,
                }, None, ROLE)?;
                direct.current_command_id = Some(prepare.id);
                direct.preparation_failed = false;
                tx.execute("UPDATE directs SET pending_turn=?1 WHERE id=?2",
                    params![serde_json::to_string(&turn)?, id.to_string()])?;
            } else {
                enqueue_turn(tx, &mut direct, turn)?;
            }
            save(tx, &direct)?;
            event(tx, "direct.prompted", id.to_string(), &direct)?;
            Ok(direct)
        })
    }

    pub fn set_direct_archived(&self, id: CommandId, body: SetDirectArchived) -> Result<Direct> {
        self.mutate(body.request_id, &format!("direct.archive/{id}"), &body, |tx| {
            let mut direct = load(tx, id)?;
            direct.state = if !body.archived {
                DirectState::Open
            } else if busy(tx, &direct)? {
                DirectState::Archiving
            } else {
                DirectState::Archived
            };
            save(tx, &direct)?;
            event(tx, "direct.state", id.to_string(), &direct)?;
            Ok(direct)
        })
    }
}

pub(super) fn list(db: &Connection, project: Option<ProjectId>) -> Result<Vec<Direct>> {
    let mut statement = db.prepare(
        "SELECT d.data FROM directs d JOIN active_projects p ON p.id=d.project_id
         WHERE (?1 IS NULL OR d.project_id=?1) ORDER BY d.rowid DESC",
    )?;
    let rows = statement.query_map([project.map(|id| id.to_string())], |row| row.get::<_, String>(0))?;
    rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
}

fn project(tx: &Transaction<'_>, id: ProjectId) -> Result<Project> {
    let data: String = tx.query_row("SELECT data FROM active_projects WHERE id=?1",
        [id.to_string()], |row| row.get(0)).optional()?.ok_or(StoreError::NotFound)?;
    Ok(serde_json::from_str(&data)?)
}

fn load(tx: &Transaction<'_>, id: CommandId) -> Result<Direct> {
    let data: String = tx.query_row(
        "SELECT d.data FROM directs d JOIN active_projects p ON p.id=d.project_id WHERE d.id=?1",
        [id.to_string()], |row| row.get(0),
    ).optional()?.ok_or(StoreError::NotFound)?;
    Ok(serde_json::from_str(&data)?)
}

fn save(tx: &Transaction<'_>, direct: &Direct) -> Result<()> {
    tx.execute("UPDATE directs SET stream_id=?1,data=?2 WHERE id=?3", params![
        direct.stream_id.map(|id| id.to_string()), serde_json::to_string(direct)?, direct.id.to_string(),
    ])?;
    Ok(())
}

fn busy(tx: &Transaction<'_>, direct: &Direct) -> Result<bool> {
    if let Some(stream) = direct.stream_id {
        return Ok(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands WHERE stream_id=?1 AND state IN ('pending','accepted'))",
            [stream.to_string()], |row| row.get(0),
        )?);
    }
    Ok(false)
}

fn validate_turn(body: &PromptDirect) -> Result<()> {
    if (body.prompt.trim().is_empty() && body.images.is_empty() && body.files.is_empty())
        || body.prompt.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(StoreError::Invalid("Invalid agent prompt"));
    }
    validate_images(&body.images).map_err(StoreError::Invalid)?;
    validate_files(&body.files).map_err(StoreError::Invalid)?;
    Ok(())
}

fn enqueue_turn(tx: &Transaction<'_>, direct: &mut Direct, turn: Turn) -> Result<()> {
    let stream_id = direct.stream_id.ok_or(StoreError::Internal)?;
    let owner = super::work::stream(tx, stream_id)?;
    let selection = direct.selection.as_ref().ok_or(StoreError::Internal)?;
    let previous = direct.root_command_id.and(direct.current_command_id);
    let mut skills = super::skills::selected(tx, direct.project_id)?;
    let mcps = if let Some(root) = direct.root_command_id {
        let raw: String = tx.query_row("SELECT data FROM commands WHERE id=?1", [root.to_string()], |row| row.get(0))?;
        let first: RunnerCommand = serde_json::from_str(&raw)?;
        let spec = first.agent_spec.ok_or(StoreError::Internal)?;
        skills = skills.for_session_prompt(&spec).ok_or(StoreError::Invalid("Direct skills are no longer granted"))?;
        McpProfile { local: spec.local_mcps, remote: spec.remote_mcps }
    } else {
        turn.mcps.ok_or(StoreError::Internal)?
    };
    let mut command = Store::enqueue_with_pins(tx, stream_id, &EnqueueCommand {
        request_id: RequestId::new(),
        expected_revision: owner.revision,
        kind: CommandKind::RunAgent,
        prompt: Some(if turn.prompt.trim().is_empty() { "See attached files.".into() } else { turn.prompt }),
    }, previous, ROLE, selection.model.as_ref(), Some((&skills, &mcps)), None)?;
    let agent = command.agent_spec.as_mut().ok_or(StoreError::Internal)?;
    agent.files = turn.files;
    agent.images = turn.images;
    super::work::save_command(tx, &command, "pending")?;
    direct.root_command_id.get_or_insert(command.id);
    direct.current_command_id = Some(command.id);
    direct.preparation_failed = false;
    Ok(())
}

pub(super) fn completed(tx: &Transaction<'_>, command: &RunnerCommand) -> Result<()> {
    let raw: Option<(String, Option<String>)> = tx.query_row(
        "SELECT data,pending_turn FROM directs WHERE stream_id=?1",
        [command.stream_id.to_string()], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let Some((data, pending)) = raw else { return Ok(()); };
    let mut direct: Direct = serde_json::from_str(&data)?;
    if direct.current_command_id != Some(command.id) {
        return Ok(());
    }
    if command.kind == CommandKind::PrepareWorkspace {
        direct.preparation_failed = true;
        if let Some(pending) = pending.filter(|_| command.state == CommandState::Succeeded) {
            let turn = serde_json::from_str(&pending)?;
            // Settings can be revoked while the workspace is preparing. Do not
            // roll back a valid runner receipt if the queued turn now fails admission.
            tx.execute_batch("SAVEPOINT direct_start")?;
            match enqueue_turn(tx, &mut direct, turn) {
                Ok(()) => tx.execute_batch("RELEASE direct_start")?,
                Err(StoreError::Invalid(_) | StoreError::WorkConflict) => {
                    tx.execute_batch("ROLLBACK TO direct_start; RELEASE direct_start")?;
                }
                Err(error) => return Err(error),
            }
        }
        if !direct.preparation_failed {
            tx.execute("UPDATE directs SET pending_turn=NULL WHERE id=?1", [direct.id.to_string()])?;
        }
    }
    if direct.state == DirectState::Archiving && !busy(tx, &direct)? {
        direct.state = DirectState::Archived;
    }
    save(tx, &direct)?;
    event(tx, "direct.state", direct.id.to_string(), &direct)?;
    Ok(())
}

pub(super) fn intent_request(
    tx: &Transaction<'_>, command: &RunnerCommand, question: &Question,
) -> Result<Option<DirectIntent>> {
    if question.external.kind != QuestionKind::Permission || question.external.payload["action"] != INTENT_APPROVAL {
        return Ok(None);
    }
    let valid_direct: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM directs WHERE stream_id=?1 AND json_extract(data,'$.current_command_id')=?2)",
        params![command.stream_id.to_string(), command.id.to_string()], |row| row.get(0),
    )?;
    if !valid_direct || command.agent_spec.as_ref().is_none_or(|spec| spec.role != ROLE) {
        return Err(StoreError::Invalid("Only an active Direct can propose an Intent"));
    }
    let intent = DirectIntent::from_permission(&question.external).ok_or(StoreError::Invalid("Invalid Direct Intent"))?;
    validate_intent(&intent)?;
    Ok(Some(intent))
}

fn validate_intent(intent: &DirectIntent) -> Result<()> {
    line(&intent.title, 80)?;
    if intent.prompt.trim().is_empty() || intent.prompt.len() > 24 * 1024
        || intent.prompt.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
    {
        return Err(StoreError::Invalid("Invalid Direct Intent text"));
    }
    Ok(())
}

/// Both the binding and the ticket commit in the same transaction as the
/// decision. Delivery retries to OpenCode cannot create a second Intent.
pub(super) fn save_intent(
    tx: &Transaction<'_>, command: &RunnerCommand, question: &Question, intent: &DirectIntent,
) -> Result<()> {
    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM direct_intents WHERE question_id=?1)",
        [question.id.to_string()], |row| row.get(0))?;
    if exists { return Ok(()); }
    let owner = super::work::stream(tx, command.stream_id)?;
    project(tx, owner.project_id)?;
    let now = super::super::unix_ms();
    let ticket = Ticket {
        id: TicketId::new(), key: super::numbering::next_reference(tx, owner.project_id)?,
        project_id: owner.project_id, revision: Revision(1), title: intent.title.clone(),
        description: intent.prompt.clone(), acceptance_criteria: String::new(),
        prompt: Some(format!("{}\n\n{}", intent.title, intent.prompt)),
        priority: 1, status: TicketState::Backlog, draft: !intent.enqueue,
        created_unix_ms: now, updated_unix_ms: now, depends_on: vec![], images: vec![], files: vec![],
        execution_command: None, requested_session: None,
        routing_note: if intent.enqueue { "Wartet auf Orchestrator" } else { "Entwurf — noch nicht freigegeben" }.into(),
    };
    let models = super::models::for_task(tx, ticket.id, owner.project_id, None, ticket.draft)?;
    let skills = super::skills::selected(tx, owner.project_id)?;
    let optional = super::mcp::default_optional(tx, owner.project_id)?;
    let mcps = super::mcp::resolve(tx, owner.project_id, &optional)?;
    tx.execute("INSERT INTO tickets(id,project_id,revision,data) VALUES(?1,?2,?3,?4)",
        params![ticket.id.to_string(), owner.project_id.to_string(), ticket.revision.0, serde_json::to_string(&ticket)?])?;
    if !ticket.draft { super::skills::pin(tx, ticket.id, &skills)?; }
    super::mcp::pin(tx, ticket.id, mcps, optional)?;
    if let Some(models) = models { super::models::save_ticket(tx, ticket.id, &models)?; }
    tx.execute("INSERT INTO direct_intents(question_id,ticket_id) VALUES(?1,?2)",
        params![question.id.to_string(), ticket.id.to_string()])?;
    event(tx, "task.saved", ticket.id.to_string(), &ticket)
}
