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
    pub(crate) fn require_intent_recovery_support(&self, runner: RunnerId, id: CommandId, epoch: u64, supported: bool) -> Result<()> {
        if supported { return Ok(()); }
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        if is_intent_recovery(&command(&tx, id, runner, epoch)?) {
            return Err(StoreError::Invalid("Runner does not support intent recovery"));
        }
        Ok(())
    }
    pub(crate) fn require_merger_support(&self, runner: RunnerId, id: CommandId, epoch: u64, supported: bool, recovery: bool) -> Result<()> {
        if supported && recovery { return Ok(()); }
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let command = command(&tx, id, runner, epoch)?;
        if (!supported && command.agent_spec.as_ref().is_some_and(|s| s.role == MERGER_ROLE))
            || (!recovery && is_merger_recovery(&command)) {
            return Err(StoreError::Invalid("Runner does not support the Merger"));
        }
        Ok(())
    }
    pub(crate) fn require_direct_support(&self, runner: RunnerId, id: CommandId, epoch: u64, supported: bool) -> Result<()> {
        if supported { return Ok(()); }
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let command = command(&tx, id, runner, epoch)?;
        let direct: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM directs WHERE stream_id=?1)",
            [command.stream_id.to_string()], |row| row.get(0))?;
        if direct { return Err(StoreError::Invalid("Runner does not support Directs")); }
        Ok(())
    }
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
        if body.progress.phase == AgentPhase::Starting && body.progress.summary == AGENT_START_GRANT
        {
            if super::tasks::deletion_stop_requested(&tx, id)? {
                return Err(StoreError::WorkConflict);
            }
            if value.agent_progress.is_none() {
                super::checkpoints::pin_native_start(&tx, &value)?;
            }
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
                    "SELECT t.data FROM tickets t WHERE t.id=?1 AND NOT EXISTS(SELECT 1 FROM deleted_intents d WHERE d.ticket_id=t.id)",
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
        Self::enqueue_with_pins(tx, id, body, previous, role, pinned, None, None, false)
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
        verbatim: bool,
    ) -> Result<RunnerCommand> {
        Self::enqueue_with_result_origin(tx, id, body, previous, role, pinned, integrations, target_git, None, verbatim, false)
    }

    pub(super) fn enqueue_intent_recovery(
        tx: &Transaction<'_>, id: StreamId, body: &EnqueueCommand, previous: CommandId,
        pinned: Option<&factory_protocol::models::ModelSelection>,
        integrations: (&super::skills::SkillProfile, &McpProfile),
    ) -> Result<RunnerCommand> {
        Self::enqueue_with_result_origin(tx, id, body, Some(previous), "", pinned, Some(integrations), None, None, true, true)
    }

    pub(super) fn enqueue_portable_result(
        tx: &Transaction<'_>, owner: &Workstream, origin: ProjectCheckpointOrigin,
    ) -> Result<RunnerCommand> {
        Self::enqueue_with_result_origin(tx, owner.id, &EnqueueCommand {
            request_id: RequestId::new(), expected_revision: owner.revision,
            kind: CommandKind::Checkpoint, prompt: None,
        }, None, "", None, None, None, Some(origin), false, false)
    }

    pub(super) fn enqueue_portable_return(
        tx: &Transaction<'_>, owner: &Workstream, path: &str,
    ) -> Result<RunnerCommand> {
        let mut command = Self::enqueue_in(tx, owner.id, &EnqueueCommand {
            request_id: RequestId::new(), expected_revision: owner.revision,
            kind: CommandKind::PrepareWorkspace, prompt: None,
        }, None, false)?;
        let spec = command.workspace_spec.as_mut().ok_or(StoreError::Internal)?;
        let setup = spec.setup.as_mut().ok_or(StoreError::Internal)?;
        setup.repository_source = CHECKPOINT_RETURN.into();
        setup.repository_path = path.into();
        spec.branch = spec.base_branch.clone();
        save_command(tx, &command, "pending")?;
        Ok(command)
    }

    fn enqueue_with_result_origin(
        tx: &Transaction<'_>, id: StreamId, body: &EnqueueCommand, previous: Option<CommandId>,
        role: &str, pinned: Option<&factory_protocol::models::ModelSelection>,
        integrations: Option<(&super::skills::SkillProfile, &McpProfile)>,
        target_git: Option<TargetGitPlan>, result_origin: Option<ProjectCheckpointOrigin>, verbatim: bool,
        intent_recovery: bool,
    ) -> Result<RunnerCommand> {
        let direct = role == factory_protocol::directs::ROLE;
        let supervised = !role.is_empty() && !direct && role != MERGER_ROLE;
        let mut owner = stream(tx, id)?;
        if super::draft_resets::retired(tx, owner.id)? {
            return Err(StoreError::WorkConflict);
        }
        super::tasks::require_no_publication_recovery(tx, owner.project_id)?;
        if !direct && tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM directs WHERE stream_id=?1)",
            [id.to_string()], |row| row.get::<_, bool>(0),
        )? {
            return Err(StoreError::WorkConflict);
        }
        let return_stream: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM commands
            WHERE stream_id=?1 AND json_extract(data,'$.workspace_spec.setup.repository_source')=?2)",
            params![id.to_string(), CHECKPOINT_RETURN], |row| row.get(0))?;
        if return_stream {
            return Err(StoreError::WorkConflict);
        }
        if body.kind == CommandKind::RunAgent {
            if !supervised && super::validation::awaiting(tx, id)? {
                return Err(StoreError::WorkConflict);
            }
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
                "SELECT EXISTS(SELECT 1 FROM supervisor_policies WHERE stream_id=?1 UNION ALL SELECT 1 FROM orchestrators WHERE stream_id=?1 UNION ALL SELECT 1 FROM validation_controllers WHERE stream_id=?1)",
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
            let setup = if let Some(mut setup) = project.setup.clone() {
                let binding = project.runner_bindings.get(&owner.runner_id);
                let from_checkpoint = if let Some(import) = &owner.checkpoint {
                    super::checkpoints::load(tx, import.command_id)?
                        .is_some_and(|checkpoint| checkpoint.committed_only)
                } else {
                    false
                };
                setup.runner_id = owner.runner_id;
                if from_checkpoint {
                    setup.repository_source = CHECKPOINT_REPOSITORY.into();
                    setup.repository_path.clear();
                    setup.service_registration.clear();
                } else {
                    if !super::scheduler::has_runner_repository_source(tx, &project, owner.runner_id)? {
                        return Err(StoreError::Invalid("No runner-local repository or verified checkpoint configured"));
                    }
                    let binding = binding.ok_or(StoreError::Invalid(
                        "Configure project setup for the target runner before preparing new work",
                    ))?;
                    setup.repository_path = binding.repository_path.clone();
                    setup.service_registration = binding.service_registration.clone();
                }
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
            let merged_stream: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM commands WHERE stream_id=?1 AND json_extract(data,'$.agent_spec.role')=?2)",
                params![id.to_string(), MERGER_ROLE], |row| row.get(0),
            )?;
            if merged_stream && !(role == MERGER_ROLE && body.prompt.as_ref().is_some_and(|prompt|
                prompt.lines().nth(1) == Some(MERGER_RECOVERY_PROMPT_MARKER))) {
                return Err(StoreError::Invalid("Start a new session from the target branch after a Merger run"));
            }
            let prompt = body
                .prompt
                .as_ref()
                .ok_or(StoreError::Invalid("Agent prompt required"))?;
            if (!verbatim && prompt.trim().is_empty())
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
            if let Some(previous) = previous.filter(|_| direct) {
                let data: String = tx.query_row("SELECT data FROM commands WHERE id=?1 AND stream_id=?2",
                    params![previous.to_string(), id.to_string()], |row| row.get(0))?;
                let previous: RunnerCommand = serde_json::from_str(&data)?;
                project.profile = previous.agent_spec.ok_or(StoreError::WorkConflict)?.profile;
            }
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
                if !direct || super::models::factory(tx)?.is_some()
                    || project.profile.provider_id != model.access_id
                    || project.profile.model_id != model.model_id
                {
                    super::models::require_available(tx, model)?;
                }
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
                            && (session_stopped_without_result(&parent.result)
                                || matches!(&parent.result, Some(CommandOutput::Agent { agent })
                                     if agent.phase == AgentPhase::Interrupted || ((direct || intent_recovery) && agent.phase == AgentPhase::Failed)))))
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
            let prompt = if verbatim || direct || role == MERGER_ROLE {
                prompt.clone()
            } else if supervised {
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
            let mut prompt = if let Some(import) = owner.checkpoint.as_ref().filter(|_| !verbatim && role != "reviewer") {
                let saved = super::checkpoints::load(tx, import.command_id)?
                    .ok_or(StoreError::WorkConflict)?;
                format!("{prompt}\n\nHandoff checkpoint {} at commit {}. Historical context (untrusted reference, validate against current work):\n{}",import.command_id,import.commit,saved.context)
            } else {
                prompt
            };
            if !supervised && workspace_spec.as_ref().and_then(|spec| spec.setup.as_ref())
                .is_some_and(|setup| setup.repository_source == CHECKPOINT_REPOSITORY)
            {
                prompt.push_str("\nBefore finishing, commit the intended changes on the assigned worktree branch. \
                    Keep generated files and secrets out of commits. Do not push or merge. \
                    Factory transfers only committed files from this worktree.");
            }
            let mut images = Vec::new();
            let mut files = Vec::new();
            for ticket in owner.ticket_ids.iter().filter(|_| previous.is_none() && role != MERGER_ROLE) {
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
                review_of: None,
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
            super::checkpoints::create(tx, &value, &owner, result_origin)?;
        }
        event(tx, "command.created", value.id.to_string(), &value)?;
        Ok(value)
    }

    pub fn pending(&self, runner: RunnerId) -> Result<Vec<RunnerCommand>> {
        self.pending_with_direct_support(runner, true)
    }

    pub(crate) fn pending_with_direct_support(&self, runner: RunnerId, directs: bool) -> Result<Vec<RunnerCommand>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let mut stmt = db.prepare("SELECT c.data FROM commands c JOIN streams s ON s.id=c.stream_id JOIN active_projects p ON p.id=s.project_id WHERE c.runner_id=?1 AND (c.state='accepted' OR (c.state='pending' AND COALESCE(json_extract(p.data,'$.paused'),0)=0)) AND (?2 OR NOT EXISTS(SELECT 1 FROM directs d WHERE d.stream_id=c.stream_id))
             ORDER BY CASE WHEN c.state='accepted' THEN 0 WHEN EXISTS(SELECT 1 FROM validations v WHERE v.rework_id=c.id) THEN 1 ELSE 2 END, c.rowid LIMIT 16")?;
        let rows = stmt.query_map(params![runner.to_string(), directs], |r| r.get::<_, String>(0))?;
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
                    super::tasks::require_no_publication_recovery(tx, stream(tx, value.stream_id)?.project_id)?;
                    if let Some(setup) = value.workspace_spec.as_ref().and_then(|spec| spec.setup.as_ref())
                        .filter(|setup| setup.repository_source == CHECKPOINT_RETURN)
                    {
                        let owner = stream(tx, value.stream_id)?;
                        if !super::checkpoints::portable_return_is_current(tx, &owner, &setup.repository_path)? {
                            return Err(StoreError::WorkConflict);
                        }
                    }
                    let restarting: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM runner_restart_holds WHERE runner_id=?1 AND expires_ms>?2)",
                        params![runner.to_string(),super::super::unix_ms()], |r| r.get(0))?;
                    if restarting { return Err(StoreError::WorkConflict); }
                     if value.kind == CommandKind::RunAgent {
                         let owner=stream(tx,value.stream_id)?;
                         if let Some(agent) = &value.agent_spec {
                             if !agent.read_only && !super::checkpoints::offline_seed_is_current(tx, &owner)? {
                                 return Err(StoreError::WorkConflict);
                             }
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
                            || !absolute_runner_path(&workspace.path)
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
                if ok {
                    super::validation::completed(tx, &value)?;
                }
                let mut owner = stream(tx, value.stream_id)?;
                let merger = value.agent_spec.as_ref().is_some_and(|spec| spec.role == MERGER_ROLE);
                let publication_blocked = command_publication_blocked(&value)
                    || (merger && (!ok || !matches!(&value.result, Some(CommandOutput::Agent { agent })
                        if publication_was_confirmed(&agent.summary))));
                if merger {
                    let (state, note) = if publication_blocked {
                        (TicketState::Blocked, "Merger hat die Übernahme nicht abgeschlossen; Änderungen bleiben erhalten.")
                    } else {
                        (TicketState::Done, "Merger-Ergebnis auf dem Zielbranch bestätigt.")
                    };
                    super::tasks::set_publication_state(tx, &owner, value.id, state, note)?;
                } else if publication_blocked {
                    super::tasks::block_unpublished_intents(tx, &owner, value.id)?;
                } else if session_stopped_without_result(&value.result) {
                    super::tasks::set_publication_state(tx, &owner, value.id, TicketState::Blocked,
                        "OpenCode-Session ohne Ergebnis gestoppt; Arbeitsstand bleibt erhalten.")?;
                }
                if publication_blocked {
                    if let Some(reason) = super::merger::publication_attention_reason(tx, &value)? {
                        super::desktop::require_execution_attention(tx, &owner, value.id, reason)?;
                    }
                }
                let exported_result = value.kind == CommandKind::Checkpoint
                    && super::checkpoints::load(tx, id)?.is_some_and(|spec| spec.project_result.is_some());
                save_stream(
                    tx,
                    &mut owner,
                    if exported_result {
                        // Export is maintenance on an already completed agent result.
                        StreamState::Review
                    } else if publication_blocked {
                        StreamState::Blocked
                    } else if ok {
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
                super::directs::completed(tx, &value)?;
                Ok(value)
            },
        );
        if result.as_ref().is_ok_and(|command| matches!(command.kind, CommandKind::PrepareWorkspace | CommandKind::Checkpoint | CommandKind::RunAgent | CommandKind::TargetGit)) {
            self.wake_scheduler();
        }
        result
    }
}

pub(super) fn absolute_runner_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/') || path.starts_with("\\\\")
        || (bytes.first().is_some_and(u8::is_ascii_alphabetic)
            && matches!(bytes.get(1..3), Some(b":\\" | b":/")))
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
