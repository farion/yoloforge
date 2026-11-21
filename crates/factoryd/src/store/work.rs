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
fn save_command(tx: &Transaction<'_>, value: &RunnerCommand, state: &str) -> Result<()> {
    tx.execute(
        "UPDATE commands SET state=?1,data=?2 WHERE id=?3",
        params![state, serde_json::to_string(value)?, value.id.to_string()],
    )?;
    event(tx, "command.updated", value.id.to_string(), value)
}

impl Store {
    pub fn update_profile(&self, id: ProjectId, body: UpdateProfile) -> Result<Project> {
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
                line(&body.profile.provider_id, 120)?;
                line(&body.profile.model_id, 200)?;
                let data: Option<String> = tx
                    .query_row(
                        "SELECT data FROM projects WHERE id=?1",
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
        validate_agent(id, &body.progress)?;
        if value.kind != CommandKind::RunAgent
            || value.state != CommandState::Accepted
            || matches!(
                body.progress.phase,
                AgentPhase::Succeeded | AgentPhase::Failed
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
            line(&body.goal, 240)?;
            if body.ticket_ids.len() > 100 {
                return Err(StoreError::Invalid("Too many tickets"));
            }
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
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
        })
    }

    pub fn enqueue(&self, id: StreamId, body: EnqueueCommand) -> Result<RunnerCommand> {
        self.mutate(body.request_id, &format!("stream.enqueue/{id}"), &body, |tx| {
            let mut owner = stream(tx, id)?;
            if owner.revision != body.expected_revision { return Err(StoreError::RevisionConflict(owner.revision)); }
            if super::controls::load(tx,id)?.is_some_and(|c|c.state!=PauseState::Released) {return Err(StoreError::WorkConflict);}
            let busy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM commands WHERE stream_id=?1 AND state IN ('pending','accepted'))", [id.to_string()], |r| r.get(0))?;
            if busy { return Err(StoreError::WorkConflict); }
            let workspace_spec = if matches!(body.kind, CommandKind::PrepareWorkspace | CommandKind::RunAgent) {
                let data: String = tx.query_row("SELECT data FROM projects WHERE id=?1", [owner.project_id.to_string()], |r|r.get(0))?;
                let project: Project = serde_json::from_str(&data)?;
                Some(WorkspaceSpec {project_id:project.id,branch:owner.branch.clone(),base_branch:project.base_branch})
            } else { None };
            let agent_spec = if body.kind == CommandKind::RunAgent {
                let prompt=body.prompt.as_ref().ok_or(StoreError::Invalid("Agent prompt required"))?;
                if prompt.trim().is_empty() || prompt.len()>16384 || prompt.chars().any(|c|c.is_control() && !matches!(c,'\n'|'\t'|'\r')) {
                    return Err(StoreError::Invalid("Invalid agent prompt"));
                }
                let prepared:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands WHERE stream_id=?1 AND state='succeeded' AND json_extract(data,'$.kind')='prepare_workspace')",[id.to_string()],|r|r.get(0))?;
                if !prepared {return Err(StoreError::Invalid("Prepare workspace before starting an agent"));}
                let data:String=tx.query_row("SELECT data FROM projects WHERE id=?1",[owner.project_id.to_string()],|r|r.get(0))?;
                let project:Project=serde_json::from_str(&data)?;
                line(&project.profile.provider_id,120)?;line(&project.profile.model_id,200)?;
                Some(AgentSpec {prompt:format!("{prompt}\n\nProject execution profile:\nToolchain: {}\nStart command: {}\nTest command: {}\n\nWork only in the assigned worktree. Preserve existing user changes. Do not push or merge. Report changes and test results.",project.profile.toolchain,project.profile.start_command,project.profile.test_command),profile:project.profile})
            } else {
                if body.prompt.is_some() {return Err(StoreError::Invalid("Prompt only valid for agent commands"));}
                None
            };
            let value = RunnerCommand {id: CommandId::new(), stream_id: id, runner_id: owner.runner_id,
                ownership_epoch: owner.ownership_epoch, kind: body.kind, state: CommandState::Pending, result: None,
                workspace_spec,
                agent_spec, agent_progress:None,
                created_unix_ms: super::super::unix_ms()};
            tx.execute("INSERT INTO commands(id,stream_id,runner_id,state,data) VALUES(?1,?2,?3,'pending',?4)",
                params![value.id.to_string(), id.to_string(), value.runner_id.to_string(), serde_json::to_string(&value)?])?;
            save_stream(tx, &mut owner, StreamState::Queued)?;
            event(tx, "command.created", value.id.to_string(), &value)?;
            Ok(value)
        })
    }

    pub fn pending(&self, runner: RunnerId) -> Result<Vec<RunnerCommand>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let mut stmt = db.prepare("SELECT data FROM commands WHERE runner_id=?1 AND state IN ('pending','accepted') ORDER BY rowid LIMIT 16")?;
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
                    value.state = CommandState::Accepted;
                    save_command(tx, &value, "accepted")?;
                    let mut owner = stream(tx, value.stream_id)?;
                    save_stream(tx, &mut owner, StreamState::Running)?;
                }
                Ok(value)
            },
        )
    }

    pub fn complete(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: CommandResult,
    ) -> Result<RunnerCommand> {
        self.mutate(
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
                let ok = match (&value.kind, &body.result) {
                    (CommandKind::RunAgent, CommandOutput::Agent { agent }) => {
                        validate_agent(id, agent)?;
                        if !matches!(agent.phase, AgentPhase::Succeeded | AgentPhase::Failed) {
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
                        if value.kind == CommandKind::RunAgent {
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
        )
    }
}

fn validate_agent(id: CommandId, progress: &AgentProgress) -> Result<()> {
    if progress.session_id != agent_session_id(id)
        || progress.message_id != agent_message_id(id)
        || progress.summary.len() > 32768
    {
        return Err(StoreError::Invalid("Invalid agent progress"));
    }
    Ok(())
}
