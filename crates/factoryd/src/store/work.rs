use super::*;

pub(super) fn stream(tx: &Transaction<'_>, id: StreamId) -> Result<Workstream> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM streams WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?)
}
pub(super) fn save_stream(
    tx: &Transaction<'_>,
    value: &mut Workstream,
    state: StreamState,
) -> Result<()> {
    let state = super::controls::stream_status(tx, value.id, state)?;
    value.revision.0 = value
        .revision
        .0
        .checked_add(1)
        .ok_or(StoreError::Internal)?;
    value.status = state;
    tx.execute(
        "UPDATE streams SET data=?1 WHERE id=?2",
        params![serde_json::to_string(value)?, value.id.to_string()],
    )?;
    event(tx, "stream.updated", value.id.to_string(), value)
}
pub(super) fn command(
    tx: &Transaction<'_>,
    id: CommandId,
    runner: RunnerId,
    epoch: u64,
) -> Result<RunnerCommand> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM commands WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let value: RunnerCommand = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
    let owner = stream(tx, value.stream_id)?;
    if value.runner_id != runner
        || owner.runner_id != runner
        || value.ownership_epoch != epoch
        || owner.ownership_epoch != epoch
    {
        return Err(StoreError::WorkConflict);
    }
    Ok(value)
}
pub(super) fn save_command(tx: &Transaction<'_>, value: &RunnerCommand, state: &str) -> Result<()> {
    tx.execute(
        "UPDATE commands SET state=?1,data=?2 WHERE id=?3",
        params![state, serde_json::to_string(value)?, value.id.to_string()],
    )?;
    event(tx, "command.updated", value.id.to_string(), value)
}

impl Store {
    pub fn require_context_planning_support(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        supported: bool,
    ) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let c = command(&tx, id, runner, epoch)?;
        if !supported
            && c.agent_spec
                .as_ref()
                .is_some_and(|s| s.profile.context_planning.is_some())
        {
            return Err(StoreError::Invalid(
                "Runner lacks context-planning-v1 capability",
            ));
        }
        Ok(())
    }
    pub fn update_profile(&self, id: ProjectId, body: UpdateProfile) -> Result<Project> {
        super::knowledge::validate_plan(&body.profile)?;
        self.mutate(
            body.request_id,
            &format!("project.profile/{id}"),
            &body,
            |tx| {
                for text in [
                    &body.profile.toolchain,
                    &body.profile.start_command,
                    &body.profile.test_command,
                ] {
                    if text.len() > 2048
                        || text
                            .chars()
                            .any(|c| c.is_control() && c != '\n' && c != '\t')
                    {
                        return Err(StoreError::Invalid("Invalid execution profile text"));
                    }
                }
                if !body.profile.provider_id.is_empty() || !body.profile.model_id.is_empty() {
                    line(&body.profile.provider_id, 120)?;
                    line(&body.profile.model_id, 200)?;
                }
                let data: Option<String> = tx
                    .query_row(
                        "SELECT data FROM active_projects WHERE id=?1",
                        [id.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?;
                let mut value: Project = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if value.revision != body.expected_revision {
                    return Err(StoreError::RevisionConflict(value.revision));
                }
                value.profile = body.profile.clone();
                value.revision.0 = value
                    .revision
                    .0
                    .checked_add(1)
                    .ok_or(StoreError::Internal)?;
                tx.execute(
                    "UPDATE projects SET data=?1 WHERE id=?2",
                    params![serde_json::to_string(&value)?, id.to_string()],
                )?;
                event(tx, "project.updated", id.to_string(), &value)?;
                Ok(value)
            },
        )
    }

    pub fn observe_agent(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: ObserveAgent,
    ) -> Result<RunnerCommand> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = command(&tx, id, runner, body.ownership_epoch)?;
        validate_agent(&value, &body.progress)?;
        if value.kind != CommandKind::RunAgent
            || value.state != CommandState::Accepted
            || matches!(
                body.progress.phase,
                AgentPhase::Succeeded | AgentPhase::Failed | AgentPhase::Interrupted
            )
        {
            return Err(StoreError::WorkConflict);
        }
        if value.agent_progress.as_ref() != Some(&body.progress) {
            let status = match body.progress.phase {
                AgentPhase::WaitingInput => StreamState::WaitingInput,
                AgentPhase::Unknown => StreamState::Blocked,
                _ => StreamState::Running,
            };
            value.agent_progress = Some(body.progress);
            save_command(&tx, &value, "accepted")?;
            let mut owner = stream(&tx, value.stream_id)?;
            save_stream(&tx, &mut owner, status)?;
        }
        tx.commit()?;
        Ok(value)
    }

    pub fn create_stream(&self, body: CreateStream) -> Result<Workstream> {
        self.mutate(body.request_id, "stream.create", &body, |tx| {
            Self::create_stream_in(tx, &body)
        })
    }
    pub(super) fn create_stream_in(
        tx: &Transaction<'_>,
        body: &CreateStream,
    ) -> Result<Workstream> {
        line(&body.goal, 240)?;
        if body.ticket_ids.len() > 100 {
            return Err(StoreError::Invalid("Too many tickets"));
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM active_projects WHERE id=?1)",
            [body.project_id.to_string()],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound);
        }
        let mut unique = std::collections::HashSet::new();
        for id in &body.ticket_ids {
            if !unique.insert(*id) {
                return Err(StoreError::Invalid("Duplicate ticket"));
            }
        }
        for id in &body.ticket_ids {
            let data: String = tx
                .query_row(
                    "SELECT data FROM tickets WHERE id=?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(StoreError::NotFound)?;
            let ticket: Ticket = serde_json::from_str(&data)?;
            if ticket.draft {
                return Err(StoreError::Invalid(
                    "Entwurf muss zuerst freigegeben werden",
                ));
            }
            let project: Option<String> = tx
                .query_row(
                    "SELECT project_id FROM tickets WHERE id=?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if project.as_deref() != Some(&body.project_id.to_string()) {
                return Err(StoreError::Invalid("Ticket does not belong to project"));
            }
            let assigned: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM stream_tickets WHERE ticket_id=?1)",
                [id.to_string()],
                |r| r.get(0),
            )?;
            if assigned {
                return Err(StoreError::WorkConflict);
            }
        }
        let id = StreamId::new();
        let value = Workstream {
            checkpoint: None,
            id,
            project_id: body.project_id,
            revision: Revision(1),
            goal: body.goal.clone(),
            runner_id: body.runner_id,
            ownership_epoch: 1,
            branch: format!("factory/{id}"),
            ticket_ids: body.ticket_ids.clone(),
            status: StreamState::Idle,
            created_unix_ms: super::super::unix_ms(),
        };
        tx.execute(
            "INSERT INTO streams(id,project_id,data) VALUES(?1,?2,?3)",
            params![
                id.to_string(),
                value.project_id.to_string(),
                serde_json::to_string(&value)?
            ],
        )?;
        for ticket in &value.ticket_ids {
            tx.execute(
                "INSERT INTO stream_tickets(ticket_id,stream_id) VALUES(?1,?2)",
                params![ticket.to_string(), id.to_string()],
            )?;
        }
        event(tx, "stream.created", id.to_string(), &value)?;
        Ok(value)
    }

    pub fn enqueue(&self, id: StreamId, body: EnqueueCommand) -> Result<RunnerCommand> {
        if matches!(
            body.kind,
            CommandKind::ReviewCheckpoint | CommandKind::IntegrateReview | CommandKind::TargetGit
        ) {
            return Err(StoreError::Invalid("Command requires a review or orchestrated Git plan"));
        }
        self.mutate(
            body.request_id,
            &format!("stream.enqueue/{id}"),
            &body,
            |tx| Self::enqueue_in(tx, id, &body, None, false),
        )
    }

    pub fn continue_agent(&self, id: StreamId, body: ContinueAgent) -> Result<RunnerCommand> {
        self.mutate(
            body.request_id,
            &format!("stream.continue/{id}"),
            &body,
            |tx| {
                Self::enqueue_in(
                    tx,
                    id,
                    &EnqueueCommand {
                        request_id: body.request_id,
                        expected_revision: body.expected_revision,
                        kind: CommandKind::RunAgent,
                        prompt: Some(body.prompt.clone()),
                    },
                    Some(body.previous_command_id),
                    false,
                )
            },
        )
    }

    pub(super) fn enqueue_in(
        tx: &Transaction<'_>,
        id: StreamId,
        body: &EnqueueCommand,
        previous: Option<CommandId>,
        supervised: bool,
    ) -> Result<RunnerCommand> {
        Self::enqueue_role(
            tx,
            id,
            body,
            previous,
            if supervised { "supervisor" } else { "" },
        )
    }
    pub(super) fn enqueue_role(
        tx: &Transaction<'_>,
        id: StreamId,
        body: &EnqueueCommand,
        previous: Option<CommandId>,
        role: &str,
    ) -> Result<RunnerCommand> {
        Self::enqueue_with_model(tx, id, body, previous, role, None)
    }

    pub(super) fn enqueue_with_model(
        tx: &Transaction<'_>,
        id: StreamId,
        body: &EnqueueCommand,
        previous: Option<CommandId>,
        role: &str,
        pinned: Option<&factory_protocol::models::ModelSelection>,
    ) -> Result<RunnerCommand> {
        Self::enqueue_with_pins(tx, id, body, previous, role, pinned, None, None)
    }

    pub(super) fn enqueue_with_pins(
        tx: &Transaction<'_>,
        id: StreamId,
        body: &EnqueueCommand,
        previous: Option<CommandId>,
        role: &str,
        pinned: Option<&factory_protocol::models::ModelSelection>,
        integrations: Option<(&super::skills::SkillProfile, &McpProfile)>,
        target_git: Option<TargetGitPlan>,
    ) -> Result<RunnerCommand> {
        let supervised = !role.is_empty();
        let mut owner = stream(tx, id)?;
        if body.kind == CommandKind::RunAgent {
            super::token_budgets::admit(tx, owner.project_id, role)?;
        }
        let paused: bool = tx.query_row(
            "SELECT COALESCE(json_extract(data,'$.paused'),0) FROM active_projects WHERE id=?1",
            [owner.project_id.to_string()],
            |r| r.get(0),
        )?;
        if paused {
            return Err(StoreError::WorkConflict);
        }
        if super::termination::active(tx, id)? {
            return Err(StoreError::WorkConflict);
        }
        if !supervised {
            let reserved: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM supervisor_policies WHERE stream_id=?1 UNION ALL SELECT 1 FROM orchestrators WHERE stream_id=?1)",
                [id.to_string()],
                |r| r.get(0),
            )?;
            if reserved {
                return Err(StoreError::WorkConflict);
            }
        }
        if owner.revision != body.expected_revision {
            return Err(StoreError::RevisionConflict(owner.revision));
        }
        if super::controls::load(tx, id)?.is_some_and(|c| c.state != PauseState::Released) {
            return Err(StoreError::WorkConflict);
        }
        let busy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM commands WHERE stream_id=?1 AND state IN ('pending','accepted'))", [id.to_string()], |r| r.get(0))?;
        if busy {
            return Err(StoreError::WorkConflict);
        }
        let mut workspace_spec = if matches!(
            body.kind,
            CommandKind::PrepareWorkspace
                | CommandKind::RunAgent
                | CommandKind::Checkpoint
                | CommandKind::ReviewCheckpoint
                | CommandKind::IntegrateReview
                | CommandKind::TargetGit
        ) {
            let data: String = tx.query_row(
                "SELECT data FROM active_projects WHERE id=?1",
                [owner.project_id.to_string()],
                |r| r.get(0),
            )?;
            let project: Project = serde_json::from_str(&data)?;
            let setup = if let Some(mut setup) = project.setup {
                let binding =
                    project
                        .runner_bindings
                        .get(&owner.runner_id)
                        .ok_or(StoreError::Invalid(
                        "Configure project setup for the target runner before preparing new work",
                    ))?;
                setup.runner_id = owner.runner_id;
                setup.repository_path = binding.repository_path.clone();
                setup.service_registration = binding.service_registration.clone();
                Some(setup)
            } else {
                None
            };
            Some(WorkspaceSpec {
                target_git: None,
                setup,
                checkpoint: owner.checkpoint.clone(),
                project_id: project.id,
                branch: owner.branch.clone(),
                base_branch: project.base_branch,
            })
        } else {
            None
        };
        if workspace_spec.is_some() && body.kind != CommandKind::PrepareWorkspace {
            let prepared:Option<String>=tx.query_row("SELECT data FROM commands WHERE stream_id=?1 AND state='succeeded' AND json_extract(data,'$.kind')='prepare_workspace' AND json_extract(data,'$.ownership_epoch')=?2 ORDER BY rowid DESC LIMIT 1",params![id.to_string(),owner.ownership_epoch],|r|r.get(0)).optional()?;
            if let Some(data) = prepared {
                let c: RunnerCommand = serde_json::from_str(&data)?;
                if let Some(spec) = c.workspace_spec {
                    workspace_spec = Some(spec);
                }
            }
        }
        if let Some(plan) = target_git {
            let spec = workspace_spec.as_mut().ok_or(StoreError::Internal)?;
            if !plan.valid() || !spec.setup.as_ref().is_some_and(ProjectSetup::has_local_target) {
                return Err(StoreError::Invalid("Target Git requires a local project checkout and a valid plan"));
            }
            spec.target_git = Some(plan);
        } else if body.kind == CommandKind::TargetGit {
            return Err(StoreError::Invalid("Target Git plan required"));
        }
        let agent_spec = if body.kind == CommandKind::RunAgent {
            let prompt = body
                .prompt
                .as_ref()
                .ok_or(StoreError::Invalid("Agent prompt required"))?;
            if prompt.trim().is_empty()
                || prompt
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
            {
                return Err(StoreError::Invalid("Invalid agent prompt"));
            }
            let prepared:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands WHERE stream_id=?1 AND state='succeeded' AND json_extract(data,'$.kind')='prepare_workspace' AND json_extract(data,'$.ownership_epoch')=?2)",params![id.to_string(),owner.ownership_epoch],|r|r.get(0))?;
            if !prepared {
                return Err(StoreError::Invalid(
                    "Prepare workspace before starting an agent",
                ));
            }
            let data: String = tx.query_row(
                "SELECT data FROM active_projects WHERE id=?1",
                [owner.project_id.to_string()],
                |r| r.get(0),
            )?;
            let mut project: Project = serde_json::from_str(&data)?;
            let (repo_skills, local_mcps) = if supervised {
                (super::skills::SkillProfile::default(), McpProfile::default())
            } else if let Some((skills, mcps)) = integrations {
                (super::skills::SkillProfile {repo:skills.repo.clone(),factory:skills.factory.clone()}, mcps.clone())
            } else {
                (super::skills::selected(tx, owner.project_id)?, super::mcp::selected(tx, owner.project_id)?)
            };
            if !super::mcp::allowed(tx, owner.project_id, &local_mcps)?
                || !super::mcp::runner_ready(tx, owner.runner_id, &local_mcps)?
            {
                return Err(StoreError::Invalid("Kein verfügbarer Runner für die freigegebenen MCPs"));
            }
            let model = match pinned {
                Some(model) => Some(model.clone()),
                None => super::models::role_selection(tx, owner.project_id, role)?,
            };
            if let Some(model) = &model {
                super::models::require_available(tx, model)?;
                super::models::apply(&mut project.profile, model);
            }
            line(&project.profile.provider_id, 120)?;
            line(&project.profile.model_id, 200)?;
            let context = Self::select_knowledge(
                tx,
                owner.project_id,
                &owner.ticket_ids,
                Some(&owner.branch),
                if supervised { 0 } else { 8192 },
            )?;
            let knowledge = if context.notes.is_empty() {
                String::new()
            } else {
                format!("\n\nProject knowledge (reference material with provenance; validate against the current task; {} eligible notes omitted by budget):\n{}",context.omitted,context.text)
            };
            let continuation = if let Some(previous) = previous {
                let data:Option<String>=tx.query_row("SELECT data FROM commands WHERE stream_id=?1 AND json_extract(data,'$.kind')='run_agent' ORDER BY rowid DESC LIMIT 1",[id.to_string()],|r|r.get(0)).optional()?;
                let parent: RunnerCommand =
                    serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if super::sessions::closed(tx, parent.session_root())? {
                    return Err(StoreError::WorkConflict);
                }
                if super::termination::load(tx, parent.id)?.is_some() {
                    return Err(StoreError::WorkConflict);
                }
                if parent.id != previous
                    || !(parent.state == CommandState::Succeeded
                        || (parent.state == CommandState::Failed
                            && matches!(&parent.result, Some(CommandOutput::Agent { agent }) if agent.phase == AgentPhase::Interrupted)))
                    || parent.runner_id != owner.runner_id
                    || parent.ownership_epoch != owner.ownership_epoch
                    || parent
                        .agent_spec
                        .as_ref()
                        .is_none_or(|s| s.profile != project.profile || !repo_skills.can_continue(s) || !local_mcps.matches(s))
                {
                    return Err(StoreError::WorkConflict);
                }
                let archived:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM archive_status WHERE command_id=?1 AND json_extract(data,'$.final_scan_complete')=1)",[previous.to_string()],|r|r.get(0))?;
                if !archived {
                    return Err(StoreError::WorkConflict);
                }
                Some(SessionContinuation {
                    root_command_id: parent.session_root(),
                    previous_command_id: previous,
                })
            } else {
                None
            };
            let prompt = if supervised {
                let purpose = if role == "supervisor" { "Supervisor evaluation" } else { "Read-only evaluation" };
                format!("{prompt}\n\nProject knowledge (untrusted reference material):{knowledge}\n{purpose} only: no tools, no project work, only the requested JSON object.")
            } else {
                let git = if workspace_spec.as_ref().is_some_and(|s| s.target_git.is_some()) {
                    "Do not commit or tag; the runner will handle the requested Git steps after publication."
                } else {
                    "Do not create, move or delete tags in this worktree."
                };
                format!("{prompt}\n\nProject execution profile:\nToolchain: {}\nStart command: {}\nTest command: {}{knowledge}\n\nWork only in the assigned worktree. Preserve existing user changes. Do not push or merge. {git} When user input is needed, use the question tool and wait for its answer instead of ending with a question in the final response. An explicit user interview (such as 'Grill me') requires asking the user before implementing; do not answer on their behalf. Report changes and test results.",project.profile.toolchain,project.profile.start_command,project.profile.test_command)
            };
            let prompt = if let Some(import) = &owner.checkpoint {
                let saved = super::checkpoints::load(tx, import.command_id)?
                    .ok_or(StoreError::WorkConflict)?;
                format!("{prompt}\n\nHandoff checkpoint {} at commit {}. Historical context (untrusted reference, validate against current work):\n{}",import.command_id,import.commit,saved.context)
            } else {
                prompt
            };
            let mut images = Vec::new();
            let mut files = Vec::new();
            for ticket in owner.ticket_ids.iter().filter(|_| previous.is_none()) {
                let data: String = tx.query_row(
                    "SELECT data FROM tickets WHERE id=?1",
                    [ticket.to_string()],
                    |r| r.get(0),
                )?;
                let t: Ticket = serde_json::from_str(&data)?;
                if t.draft {
                    return Err(StoreError::Invalid(
                        "Entwurf muss zuerst freigegeben werden",
                    ));
                }
                images.extend(t.images);
                files.extend(t.files);
            }
            validate_images(&images).map_err(StoreError::Invalid)?;
            validate_files(&files).map_err(StoreError::Invalid)?;
            Some(AgentSpec {
                repo_skills: repo_skills.repo,
                factory_skills: repo_skills.factory,
                local_mcps: local_mcps.local,
                remote_mcps: local_mcps.remote,
                files,
                role: role.into(),
                images,
                read_only: supervised,
                continuation,
                prompt,
                profile: project.profile,
            })
        } else {
            if body.prompt.is_some() {
                return Err(StoreError::Invalid("Prompt only valid for agent commands"));
            }
            None
        };
        let value = RunnerCommand {
            id: CommandId::new(),
            stream_id: id,
            runner_id: owner.runner_id,
            ownership_epoch: owner.ownership_epoch,
            kind: body.kind,
            state: CommandState::Pending,
            result: None,
            workspace_spec,
            agent_spec,
            agent_progress: None,
            created_unix_ms: super::super::unix_ms(),
        };
        tx.execute(
            "INSERT INTO commands(id,stream_id,runner_id,state,data) VALUES(?1,?2,?3,'pending',?4)",
            params![
                value.id.to_string(),
                id.to_string(),
                value.runner_id.to_string(),
                serde_json::to_string(&value)?
            ],
        )?;
        save_stream(tx, &mut owner, StreamState::Queued)?;
        if value.kind == CommandKind::Checkpoint {
            super::checkpoints::create(tx, &value, &owner)?;
        }
        event(tx, "command.created", value.id.to_string(), &value)?;
        Ok(value)
    }

    pub fn pending(&self, runner: RunnerId) -> Result<Vec<RunnerCommand>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let mut stmt = db.prepare("SELECT c.data FROM commands c JOIN streams s ON s.id=c.stream_id JOIN active_projects p ON p.id=s.project_id WHERE c.runner_id=?1 AND (c.state='accepted' OR (c.state='pending' AND COALESCE(json_extract(p.data,'$.paused'),0)=0)) ORDER BY c.rowid LIMIT 16")?;
        let rows = stmt.query_map([runner.to_string()], |r| r.get::<_, String>(0))?;
        let mut values = Vec::new();
        for row in rows {
            values.push(serde_json::from_str(&row?)?);
        }
        Ok(values)
    }

    pub fn acknowledge(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: AckCommand,
    ) -> Result<RunnerCommand> {
        self.mutate(
            body.request_id,
            &format!("command.ack/{runner}/{id}"),
            &body,
            |tx| {
                let mut value = command(tx, id, runner, body.ownership_epoch)?;
                if value.state == CommandState::Pending {
                    let restarting: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM runner_restart_holds WHERE runner_id=?1 AND expires_ms>?2)",
                        params![runner.to_string(),super::super::unix_ms()], |r| r.get(0))?;
                    if restarting { return Err(StoreError::WorkConflict); }
                     if value.kind == CommandKind::RunAgent {
                         let owner=stream(tx,value.stream_id)?;
                         if let Some(agent) = &value.agent_spec {
                             let profile = McpProfile::from_agent(agent);
                             if !super::mcp::allowed(tx, owner.project_id, &profile)?
                                 || !super::mcp::runner_ready(tx, runner, &profile)?
                             {
                                 return Err(StoreError::WorkConflict);
                             }
                         }
                        super::token_budgets::admit(tx,owner.project_id,value.agent_spec.as_ref().map_or("",|a|a.role.as_str()))
                            .map_err(|e| match e { StoreError::Invalid(_) => StoreError::WorkConflict, other => other })?;
                    }
                    let paused:bool=tx.query_row("SELECT COALESCE(json_extract(p.data,'$.paused'),0) FROM active_projects p JOIN streams s ON s.project_id=p.id WHERE s.id=?1",[value.stream_id.to_string()],|r|r.get(0))?;if paused{return Err(StoreError::WorkConflict);}
                    if value.kind==CommandKind::RunAgent && value.agent_spec.as_ref().is_some_and(|a|!a.read_only) {
                        let owner=stream(tx,value.stream_id)?;
                        let limit = tx.query_row(
                            "SELECT json_extract(data,'$.max_sessions') FROM active_projects WHERE id=?1",
                            [owner.project_id.to_string()],
                            |r| r.get::<_, Option<u8>>(0),
                        )?.unwrap_or_else(default_session_limit);
                        let active:u64=tx.query_row("SELECT COUNT(DISTINCT c.stream_id) FROM commands c JOIN streams s ON s.id=c.stream_id WHERE s.project_id=?1 AND c.state='accepted' AND json_extract(c.data,'$.kind')='run_agent' AND COALESCE(json_extract(c.data,'$.agent_spec.read_only'),0)=0",[owner.project_id.to_string()],|r|r.get(0))?;
                        if active>=u64::from(limit) {return Err(StoreError::WorkConflict);}
                    }
                    value.state = CommandState::Accepted;
                    save_command(tx, &value, "accepted")?;
                    let mut owner = stream(tx, value.stream_id)?;
                    save_stream(tx, &mut owner, StreamState::Running)?;
                }
                Ok(value)
            },
        )
    }

    pub fn require_target_git_support(&self, runner: RunnerId, id: CommandId, epoch: u64, supported: bool) -> Result<()> {
        if !supported {
            let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
            let tx = db.transaction()?;
            if command(&tx, id, runner, epoch)?.requires_target_git() {
                return Err(StoreError::WorkConflict);
            }
        }
        Ok(())
    }

    /// Check immutable command capabilities even for an idempotent acknowledgement replay.
    pub fn acknowledge_supported(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: AckCommand,
        lifecycle: bool,
    ) -> Result<RunnerCommand> {
        if !lifecycle {
            let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
            let tx = db.transaction()?;
            if command(&tx, id, runner, body.ownership_epoch)?.requires_session_lifecycle() {
                return Err(StoreError::WorkConflict);
            }
        }
        self.acknowledge(runner, id, body)
    }

    pub fn complete(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: CommandResult,
    ) -> Result<RunnerCommand> {
        let result = self.mutate(
            body.request_id,
            &format!("command.result/{runner}/{id}"),
            &body,
            |tx| {
                let mut value = command(tx, id, runner, body.ownership_epoch)?;
                if matches!(value.state, CommandState::Succeeded | CommandState::Failed) {
                    if value.result.as_ref() != Some(&body.result) {
                        return Err(StoreError::WorkConflict);
                    }
                    return Ok(value);
                }
                if value.state != CommandState::Accepted {
                    return Err(StoreError::WorkConflict);
                }
                if super::termination::active(tx, value.stream_id)? {
                    return Err(StoreError::WorkConflict);
                }
                if super::controls::load(tx, value.stream_id)?.is_some_and(|c| {
                    c.state != PauseState::Released
                        && (c.proof.is_some() || c.state == PauseState::ResumeRequested)
                }) {
                    return Err(StoreError::WorkConflict);
                }
                if super::compaction::latest(tx, id)?.is_some_and(|v| !v.state.terminal()) {
                    return Err(StoreError::WorkConflict);
                }
                let ok = match (&value.kind, &body.result) {
                    (CommandKind::TargetGit, CommandOutput::TargetGit { git_summary, git_succeeded }) => {
                        line(git_summary, 1024)?;
                        *git_succeeded
                    }
                    (CommandKind::ReviewCheckpoint | CommandKind::IntegrateReview, output) => {
                        super::reviews::validate(tx, &value, output)?
                    }
                    (CommandKind::Checkpoint, CommandOutput::Checkpoint { checkpoint }) => {
                        super::checkpoints::validate(checkpoint)?;
                        super::checkpoints::verify_bundle(tx, id, checkpoint)?;
                        true
                    }
                    (
                        CommandKind::Checkpoint,
                        CommandOutput::CheckpointError { checkpoint_error },
                    ) => {
                        line(checkpoint_error, 240)?;
                        false
                    }
                    (CommandKind::RunAgent, CommandOutput::Agent { agent }) => {
                        validate_agent(&value, agent)?;
                        if !matches!(
                            agent.phase,
                            AgentPhase::Succeeded | AgentPhase::Failed | AgentPhase::Interrupted
                        ) {
                            return Err(StoreError::WorkConflict);
                        }
                        if agent.phase == AgentPhase::Interrupted
                            && super::controls::load(tx, value.stream_id)?.is_none_or(|c| {
                                c.command_id != Some(id)
                                    || c.state != PauseState::SessionInterrupted
                            })
                        {
                            return Err(StoreError::WorkConflict);
                        }
                        value.agent_progress = Some(agent.clone());
                        agent.phase == AgentPhase::Succeeded
                    }
                    (CommandKind::CheckOpenCode, CommandOutput::OpenCode(status)) => {
                        matches!(status, OpenCodeStatus::Ready { .. })
                    }
                    (CommandKind::PrepareWorkspace, CommandOutput::Workspace { workspace }) => {
                        if value
                            .workspace_spec
                            .as_ref()
                            .and_then(|s| s.checkpoint.as_ref())
                            .is_some_and(|c| c.commit != workspace.base_commit)
                        {
                            return Err(StoreError::WorkConflict);
                        }
                        if value
                            .workspace_spec
                            .as_ref()
                            .is_none_or(|s| s.branch != workspace.branch)
                            || !workspace.path.starts_with('/')
                            || workspace.path.len() > 4096
                            || workspace.path.chars().any(char::is_control)
                            || !matches!(workspace.base_commit.len(), 40 | 64)
                            || !workspace.base_commit.bytes().all(|b| b.is_ascii_hexdigit())
                        {
                            return Err(StoreError::Invalid("Invalid workspace result"));
                        }
                        true
                    }
                    (
                        CommandKind::PrepareWorkspace,
                        CommandOutput::WorkspaceError { workspace_error },
                    ) => {
                        line(workspace_error, 240)?;
                        false
                    }
                    _ => return Err(StoreError::Invalid("Result does not match command kind")),
                };
                value.state = if ok {
                    CommandState::Succeeded
                } else {
                    CommandState::Failed
                };
                value.result = Some(body.result.clone());
                super::controls::completed(tx, value.stream_id, id)?;
                super::questions::close(tx, id)?;
                save_command(tx, &value, if ok { "succeeded" } else { "failed" })?;
                let mut owner = stream(tx, value.stream_id)?;
                save_stream(
                    tx,
                    &mut owner,
                    if ok {
                        if matches!(
                            value.kind,
                            CommandKind::RunAgent | CommandKind::ReviewCheckpoint
                        ) {
                            StreamState::Review
                        } else {
                            StreamState::Idle
                        }
                    } else {
                        StreamState::Blocked
                    },
                )?;
                Ok(value)
            },
        );
        if result.as_ref().is_ok_and(|command| matches!(command.kind, CommandKind::PrepareWorkspace | CommandKind::RunAgent | CommandKind::TargetGit)) {
            self.wake_scheduler();
        }
        result
    }
}

fn validate_agent(command: &RunnerCommand, progress: &AgentProgress) -> Result<()> {
    let id = command.id;
    if progress.session_id != command.session_id()
        || progress.message_id != agent_message_id(id)
        || progress.summary.len() > 32768
    {
        return Err(StoreError::Invalid("Invalid agent progress"));
    }
    Ok(())
}
