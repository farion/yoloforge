use super::*;
pub(super) fn binding(tx: &Transaction<'_>, id: CommandId) -> Result<Option<RuntimeBinding>> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM runtime_bindings WHERE command_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    data.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}
pub(super) fn load(tx: &Transaction<'_>, id: CommandId) -> Result<Option<RuntimeTermination>> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM runtime_terminations WHERE command_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    data.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}
pub(super) fn active(tx: &Transaction<'_>, id: StreamId) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM runtime_terminations WHERE stream_id=?1 AND state!='stopped')",
        [id.to_string()],
        |r| r.get(0),
    )?)
}
fn save(tx: &Transaction<'_>, value: &RuntimeTermination) -> Result<()> {
    let state = match value.state {
        TerminationState::Requested => "requested",
        TerminationState::Unknown => "unknown",
        TerminationState::Stopped => "stopped",
    };
    tx.execute("INSERT INTO runtime_terminations(command_id,stream_id,state,data) VALUES(?1,?2,?3,?4) ON CONFLICT(command_id) DO UPDATE SET state=excluded.state,data=excluded.data",params![value.command_id.to_string(),value.stream_id.to_string(),state,serde_json::to_string(value)?])?;
    event(
        tx,
        "runtime.termination",
        value.command_id.to_string(),
        value,
    )
}
impl Store {
    pub fn report_runtime_binding(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: RuntimeBinding,
    ) -> Result<RuntimeBinding> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let cmd = super::work::command(&tx, id, runner, body.ownership_epoch)?;
        if body.command_id != id
            || body.stream_id != cmd.stream_id
            || body.runner_id != runner
            || cmd.kind != CommandKind::RunAgent
        {
            return Err(StoreError::WorkConflict);
        }
        if let Some(old) = binding(&tx, id)? {
            return if old == body {
                Ok(old)
            } else {
                Err(StoreError::WorkConflict)
            };
        }
        if cmd.state != CommandState::Accepted || load(&tx, id)?.is_some() {
            return Err(StoreError::WorkConflict);
        }
        line(&body.runtime.started_at, 128)?;
        line(&body.daemon_id, 200)?;
        line(&body.workspace, 4096)?;
        if body.runtime.container_id.len() != 64
            || !body
                .runtime
                .container_id
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            || !body.workspace.starts_with('/')
        {
            return Err(StoreError::Invalid("Invalid runtime identity"));
        }
        let url = url::Url::parse(&body.endpoint)
            .map_err(|_| StoreError::Invalid("Invalid runtime endpoint"))?;
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(StoreError::Invalid(
                "Local credential-free endpoint required",
            ));
        }
        tx.execute(
            "INSERT INTO runtime_bindings(command_id,data) VALUES(?1,?2)",
            params![id.to_string(), serde_json::to_string(&body)?],
        )?;
        event(&tx, "runtime.bound", id.to_string(), &body)?;
        tx.commit()?;
        Ok(body)
    }
    pub fn termination(&self, id: CommandId) -> Result<Option<RuntimeTermination>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        load(&tx, id)
    }
    pub fn runner_termination(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
    ) -> Result<Option<RuntimeTermination>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        super::work::command(&tx, id, runner, epoch)?;
        load(&tx, id)
    }
    pub fn request_termination(
        &self,
        id: CommandId,
        body: RequestTermination,
    ) -> Result<RuntimeTermination> {
        self.mutate(
            body.request_id,
            &format!("runtime.terminate/{id}"),
            &body,
            |tx| Self::request_termination_in(tx, id, &body),
        )
    }
    pub(super) fn request_termination_in(
        tx: &Transaction<'_>,
        id: CommandId,
        body: &RequestTermination,
    ) -> Result<RuntimeTermination> {
        line(&body.reason, 512)?;
        let data: Option<String> = tx
            .query_row(
                "SELECT data FROM commands WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let cmd: RunnerCommand = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
        let mut owner = super::work::stream(tx, cmd.stream_id)?;
        if owner.revision != body.expected_stream_revision {
            return Err(StoreError::RevisionConflict(owner.revision));
        }
        if cmd.kind != CommandKind::RunAgent
            || cmd.state != CommandState::Accepted
            || cmd.runner_id != owner.runner_id
            || cmd.ownership_epoch != owner.ownership_epoch
        {
            return Err(StoreError::WorkConflict);
        }
        let bound = binding(tx, id)?;
        if let Some(bound) = &bound {
            if bound.runtime != body.runtime {
                return Err(StoreError::WorkConflict);
            }
        } else {
            let c = super::controls::load(tx, cmd.stream_id)?.ok_or(StoreError::WorkConflict)?;
            let p = c.proof.ok_or(StoreError::WorkConflict)?;
            if c.command_id != Some(id)
                || body.runtime.container_id != p.container_id
                || body.runtime.started_at != p.started_at
            {
                return Err(StoreError::WorkConflict);
            }
        }
        match (load(tx, id)?, body.previous_request) {
            (None, None) => {}
            (Some(old), Some(previous))
                if old.request_id == previous
                    && old.state == TerminationState::Unknown
                    && old.runtime == body.runtime => {}
            _ => return Err(StoreError::WorkConflict),
        }
        let v = RuntimeTermination {
            binding: bound,
            request_id: body.request_id,
            command_id: id,
            stream_id: cmd.stream_id,
            runner_id: cmd.runner_id,
            ownership_epoch: cmd.ownership_epoch,
            runtime: body.runtime.clone(),
            previous_request: body.previous_request,
            reason: body.reason.clone(),
            state: TerminationState::Requested,
            summary: "Removal requested; process quiescence not yet confirmed".into(),
            proof: None,
            result: None,
            archive_may_be_incomplete: true,
        };
        save(tx, &v)?;
        super::controls::termination(tx, cmd.stream_id, false)?;
        super::work::save_stream(tx, &mut owner, StreamState::Blocked)?;
        Ok(v)
    }
    pub fn report_termination(
        &self,
        runner: RunnerId,
        id: CommandId,
        body: ReportTermination,
    ) -> Result<RuntimeTermination> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut cmd = super::work::command(&tx, id, runner, body.ownership_epoch)?;
        let mut v = load(&tx, id)?.ok_or(StoreError::WorkConflict)?;
        if v.request_id != body.request_id {
            return Err(StoreError::WorkConflict);
        }
        if v.state == TerminationState::Stopped {
            if body.state == v.state && body.proof == v.proof {
                return Ok(v);
            }
            return Err(StoreError::WorkConflict);
        }
        if cmd.state != CommandState::Accepted {
            return Err(StoreError::WorkConflict);
        }
        match body.state {
            TerminationState::Unknown if body.proof.is_none() => {
                v.state = TerminationState::Unknown;
                v.summary = "Runtime identity/removal unverified; command remains reserved".into();
            }
            TerminationState::Stopped => {
                let p = body
                    .proof
                    .ok_or(StoreError::Invalid("Removal evidence required"))?;
                line(&p.daemon_id, 200)?;
                if p.runtime != v.runtime
                    || p.observed_unix_ms == 0
                    || v.binding
                        .as_ref()
                        .is_some_and(|b| b.daemon_id != p.daemon_id)
                {
                    return Err(StoreError::WorkConflict);
                }
                v.proof = Some(p);
                v.state = TerminationState::Stopped;
                v.summary="Pinned runtime removed; processes stopped. Archive may be incomplete; volumes and worktree retained.".into();
                let progress = AgentProgress {
                    session_id: cmd.session_id(),
                    message_id: agent_message_id(id),
                    phase: AgentPhase::Failed,
                    summary: v.summary.clone(),
                };
                cmd.state = CommandState::Failed;
                cmd.agent_progress = Some(progress.clone());
                cmd.result = Some(CommandOutput::Agent { agent: progress });
                v.result = cmd.result.clone();
                super::questions::close(&tx, id)?;
                super::compaction::terminate(&tx, id)?;
                super::controls::termination(&tx, cmd.stream_id, true)?;
                super::work::save_command(&tx, &cmd, "failed")?;
                let mut owner = super::work::stream(&tx, cmd.stream_id)?;
                super::work::save_stream(&tx, &mut owner, StreamState::Blocked)?;
            }
            _ => return Err(StoreError::WorkConflict),
        }
        if load(&tx, id)?.as_ref() != Some(&v) {
            save(&tx, &v)?;
        }
        tx.commit()?;
        Ok(v)
    }
}
