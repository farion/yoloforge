use super::*;
pub(super) fn load(tx: &Transaction<'_>, id: CommandId) -> Result<Option<CheckpointSpec>> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM checkpoints WHERE command_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    data.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}
pub(super) fn create(
    tx: &Transaction<'_>, cmd: &RunnerCommand, owner: &Workstream,
    project_result: Option<ProjectCheckpointOrigin>,
) -> Result<()> {
    let prepared:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands WHERE stream_id=?1 AND state='succeeded' AND json_extract(data,'$.kind')='prepare_workspace' AND json_extract(data,'$.ownership_epoch')=?2)",params![owner.id.to_string(),owner.ownership_epoch],|r|r.get(0))?;
    if !prepared {
        return Err(StoreError::WorkConflict);
    }
    let mut stmt=tx.prepare("SELECT data FROM commands WHERE stream_id=?1 AND json_extract(data,'$.kind')='run_agent' AND json_extract(data,'$.ownership_epoch')=?2 ORDER BY rowid")?;
    let commands = stmt
        .query_map(params![owner.id.to_string(), owner.ownership_epoch], |r| {
            r.get::<_, String>(0)
        })?
        .map(|v| Ok(serde_json::from_str::<RunnerCommand>(&v?)?))
        .collect::<Result<Vec<_>>>()?;
    let mut proofs = Vec::new();
    for c in &commands {
        if let Some(t) =
            super::termination::load(tx, c.id)?.filter(|t| t.state == TerminationState::Stopped)
        {
            let proof = t.proof.ok_or(StoreError::WorkConflict)?;
            if !proofs.contains(&proof) {
                proofs.push(proof);
            }
        }
    }
    for c in &commands {
        let own_stop = super::termination::load(tx, c.id)?
            .is_some_and(|t| t.state == TerminationState::Stopped);
        let bound = super::termination::binding(tx, c.id)?;
        let completed_result = project_result.is_some()
            && commands.len() == 1 && c.state == CommandState::Succeeded
            && c.agent_spec.as_ref().is_some_and(|spec| !spec.read_only && spec.continuation.is_none());
        if !own_stop && !completed_result
            && !bound.is_some_and(|b| {
                proofs
                    .iter()
                    .any(|p| p.runtime == b.runtime && p.daemon_id == b.daemon_id)
            })
        {
            return Err(StoreError::WorkConflict);
        }
    }
    let knowledge = Store::select_knowledge(
        tx,
        owner.project_id,
        &owner.ticket_ids,
        Some(&owner.branch),
        8192,
    )?;
    let data: String = tx.query_row(
        "SELECT data FROM active_projects WHERE id=?1",
        [owner.project_id.to_string()],
        |r| r.get(0),
    )?;
    let project: Project = serde_json::from_str(&data)?;
    let spec=CheckpointSpec {command_id:cmd.id,committed_only:project_result.is_some(),project_result,source_commands:commands.iter().map(|c|c.id).collect(),stopped_runtimes:proofs,context:serde_json::json!({"project_id":owner.project_id,"stream_id":owner.id,"goal":owner.goal,"ticket_ids":owner.ticket_ids,"branch":owner.branch,"predecessor_checkpoint":owner.checkpoint,"profile":project.profile,"knowledge":knowledge.text,"archives_may_be_incomplete":!commands.is_empty()}).to_string()};
    tx.execute(
        "INSERT INTO checkpoints(command_id,data) VALUES(?1,?2)",
        params![cmd.id.to_string(), serde_json::to_string(&spec)?],
    )?;
    event(tx, "checkpoint.requested", cmd.id.to_string(), &spec)
}
pub(super) fn mark_committed_only(tx: &Transaction<'_>, id: CommandId) -> Result<()> {
    let mut spec = load(tx, id)?.ok_or(StoreError::Internal)?;
    spec.committed_only = true;
    tx.execute("UPDATE checkpoints SET data=?1 WHERE command_id=?2",
        params![serde_json::to_string(&spec)?, id.to_string()])?;
    Ok(())
}

pub(super) fn automatic_baseline(
    tx: &Transaction<'_>, owner: &Workstream,
) -> Result<Option<RunnerCommand>> {
    let raw: Option<String> = tx.query_row("SELECT c.data FROM commands c
        JOIN checkpoints p ON p.command_id=c.id
        WHERE c.stream_id=?1 AND c.state='succeeded'
        AND json_extract(c.data,'$.ownership_epoch')=?2
        AND json_extract(p.data,'$.committed_only')=1
        AND json_extract(p.data,'$.project_result') IS NULL ORDER BY c.rowid DESC LIMIT 1",
        params![owner.id.to_string(), owner.ownership_epoch], |row| row.get(0)).optional()?;
    raw.map(|value| serde_json::from_str(&value).map_err(Into::into)).transpose()
}

/// Only reuse a project snapshot from its pinned source checkout. A newer
/// writer (or one still in flight) makes this snapshot unsafe for new work.
pub(super) fn offline_project_seed(
    tx: &Transaction<'_>, project: &Project, source: &ProjectSetup,
) -> Result<Option<CheckpointImport>> {
    offline_project_seed_except(tx, project, source, None)
}

fn offline_project_seed_except(
    tx: &Transaction<'_>, project: &Project, source: &ProjectSetup, completed: Option<CommandId>,
) -> Result<Option<CheckpointImport>> {
    let raw: Option<String> = tx.query_row(
        "SELECT c.data FROM commands c
         JOIN streams s ON s.id=c.stream_id
         JOIN checkpoints spec ON spec.command_id=c.id
         JOIN checkpoint_bundles blob ON blob.command_id=c.id
         JOIN events uploaded ON uploaded.aggregate_id=c.id AND uploaded.kind='checkpoint.bundle'
         WHERE s.project_id=?1 AND c.state='succeeded'
           AND json_extract(spec.data,'$.committed_only')=1
           AND (
               (c.runner_id=?2 AND json_extract(c.data,'$.workspace_spec.setup.repository_source')=?3
                AND json_extract(c.data,'$.workspace_spec.base_branch')=?4
                AND json_extract(c.data,'$.workspace_spec.setup.repository_path')=?5)
               OR (json_extract(spec.data,'$.project_result.runner_id')=?2
                   AND json_extract(spec.data,'$.project_result.repository')=?3
                   AND json_extract(spec.data,'$.project_result.base_branch')=?4
                   AND json_extract(spec.data,'$.project_result.repository_path')=?5)
           )
           AND NOT EXISTS(
               SELECT 1 FROM commands writer JOIN streams w ON w.id=writer.stream_id
                WHERE w.project_id=s.project_id AND writer.id<>?6
               AND (
                   json_extract(writer.data,'$.kind')='target_git'
                   OR (json_extract(writer.data,'$.kind')='run_agent'
                       AND COALESCE(json_extract(writer.data,'$.agent_spec.read_only'),0)=0)
               )
               AND (writer.state='accepted' OR (writer.state='succeeded' AND EXISTS(
                   SELECT 1 FROM events changed
                   WHERE changed.aggregate_id=writer.id AND changed.kind='command.updated'
                   AND changed.sequence>uploaded.sequence
               )))
           )
         ORDER BY uploaded.sequence DESC LIMIT 1",
        params![project.id.to_string(), source.runner_id.to_string(), source.repository_source,
            project.base_branch, source.repository_path, completed.map_or_else(String::new, |id| id.to_string())], |row| row.get(0),
    ).optional()?;
    let Some(raw) = raw else { return Ok(None); };
    let command: RunnerCommand = serde_json::from_str(&raw)?;
    let Some(CommandOutput::Checkpoint { checkpoint }) = command.result else {
        return Err(StoreError::Internal);
    };
    verify_bundle(tx, command.id, &checkpoint)?;
    Ok(Some(CheckpointImport {
        command_id: command.id, commit: checkpoint.commit, bundle_sha256: checkpoint.bundle_sha256,
        bundle_bytes: checkpoint.bundle_bytes,
    }))
}

pub(super) fn portable_result_origin(
    tx: &Transaction<'_>, owner: &Workstream, completed: &RunnerCommand,
) -> Result<Option<ProjectCheckpointOrigin>> {
    let direct_managed = completed.workspace_spec.as_ref().and_then(|spec| spec.setup.as_ref())
        .is_some_and(|setup| setup.repository_source == "factory:managed");
    let base = if let Some(base) = &owner.checkpoint {
        base.clone()
    } else if direct_managed {
        let Some(command) = automatic_baseline(tx, owner)? else { return Ok(None); };
        let Some(CommandOutput::Checkpoint { checkpoint }) = command.result else {
            return Err(StoreError::Internal);
        };
        verify_bundle(tx, command.id, &checkpoint)?;
        CheckpointImport {
            command_id: command.id, commit: checkpoint.commit,
            bundle_sha256: checkpoint.bundle_sha256, bundle_bytes: checkpoint.bundle_bytes,
        }
    } else {
        return Ok(None);
    };
    if completed.state != CommandState::Succeeded
        || command_publication_blocked(completed)
        || completed.workspace_spec.as_ref().is_none_or(|spec|
            spec.target_git.is_some()
                || (!direct_managed && spec.checkpoint.as_ref() != Some(&base))
                || spec.setup.as_ref().is_none_or(|setup|
                    !matches!(setup.repository_source.as_str(), CHECKPOINT_REPOSITORY | "factory:managed")))
    {
        return Ok(None);
    }
    let raw: String = tx.query_row("SELECT data FROM active_projects WHERE id=?1",
        [owner.project_id.to_string()], |row| row.get(0))?;
    let project: Project = serde_json::from_str(&raw)?;
    let Some(source) = &project.setup else { return Ok(None); };
    if project.runner_mapping != Some(ProjectRunnerMapping::Automatic)
        || offline_project_seed_except(tx, &project, source, Some(completed.id))?.as_ref() != Some(&base)
    {
        return Ok(None);
    }
    Ok(Some(ProjectCheckpointOrigin {
        runner_id: source.runner_id, repository: source.repository_source.clone(),
        repository_path: source.repository_path.clone(), base_branch: project.base_branch,
    }))
}

pub(super) fn has_unreturned_portable_result(
    tx: &Transaction<'_>, project: &Project, source: &ProjectSetup,
) -> Result<bool> {
    Ok(tx.query_row("SELECT COALESCE((
        SELECT NOT EXISTS(SELECT 1 FROM commands returned
            WHERE returned.runner_id=?2 AND returned.state='succeeded'
              AND json_extract(returned.data,'$.workspace_spec.checkpoint.command_id')=c.id
              AND json_extract(returned.data,'$.workspace_spec.setup.repository_source')=?6)
        FROM checkpoints cp JOIN commands c ON c.id=cp.command_id
        JOIN streams s ON s.id=c.stream_id WHERE s.project_id=?1
          AND json_extract(cp.data,'$.project_result.runner_id')=?2
          AND json_extract(cp.data,'$.project_result.repository')=?3
          AND json_extract(cp.data,'$.project_result.base_branch')=?4
          AND json_extract(cp.data,'$.project_result.repository_path')=?5
        ORDER BY c.rowid DESC LIMIT 1
    ),0)", params![project.id.to_string(), source.runner_id.to_string(),
        source.repository_source, project.base_branch, source.repository_path, CHECKPOINT_RETURN], |row| row.get(0))?)
}

pub(super) fn seed_stream(tx: &Transaction<'_>, owner: &mut Workstream, seed: Option<&CheckpointImport>) -> Result<()> {
    if let Some(seed) = seed {
        owner.checkpoint = Some(seed.clone());
        super::work::save_stream(tx, owner, StreamState::Idle)?;
    }
    Ok(())
}

pub(super) fn offline_seed_is_current(tx: &Transaction<'_>, owner: &Workstream) -> Result<bool> {
    let Some(seed) = owner.checkpoint.as_ref().filter(|_| owner.ownership_epoch == 1) else {
        // A same-stream ownership transfer intentionally resumes its own checkpoint.
        return Ok(true);
    };
    let raw: String = tx.query_row("SELECT data FROM active_projects WHERE id=?1",
        [owner.project_id.to_string()], |row| row.get(0))?;
    let project: Project = serde_json::from_str(&raw)?;
    let Some(source) = &project.setup else { return Ok(false); };
    Ok(project.runner_mapping == Some(ProjectRunnerMapping::Automatic)
        && offline_project_seed(tx, &project, source)?.as_ref() == Some(seed))
}

pub(super) fn portable_return_is_current(tx: &Transaction<'_>, owner: &Workstream, path: &str) -> Result<bool> {
    let raw: String = tx.query_row("SELECT data FROM active_projects WHERE id=?1",
        [owner.project_id.to_string()], |row| row.get(0))?;
    let project: Project = serde_json::from_str(&raw)?;
    if project.repository == "factory:managed" {
        let configured = project.mirrors.get(&owner.runner_id)
            .is_some_and(|mirror| mirror.path == path);
        let current = if let Some(source) = project.setup.as_ref() {
            offline_project_seed(tx, &project, source)?
        } else {
            None
        };
        return Ok(configured && current.as_ref() == owner.checkpoint.as_ref());
    }
    let Some(source) = project.setup else { return Ok(false); };
    let expected = if source.repository_path.is_empty() { &source.repository_source } else { &source.repository_path };
    Ok(source.runner_id == owner.runner_id && path == expected && offline_seed_is_current(tx, owner)?)
}

fn native_boot(tx: &Transaction<'_>, runner: RunnerId) -> Result<Option<NativeRunnerBoot>> {
    let raw: Option<String> = tx.query_row("SELECT data FROM native_runner_boots WHERE runner_id=?1",
        [runner.to_string()], |row| row.get(0)).optional()?;
    raw.map(|value| serde_json::from_str(&value).map_err(Into::into)).transpose()
}

pub(super) fn pin_native_start(tx: &Transaction<'_>, command: &RunnerCommand) -> Result<()> {
    let Some(boot) = native_boot(tx, command.runner_id)? else { return Ok(()); };
    let already_pinned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events
        WHERE aggregate_id=?1 AND kind='agent.native_start_boot')",
        [command.id.to_string()], |row| row.get(0))?;
    if !already_pinned {
        let owner = super::work::stream(tx, command.stream_id)?;
        if automatic_baseline(tx, &owner)?.is_some() {
            event(tx, "agent.native_start_boot", command.id.to_string(), &boot)?;
        }
    }
    Ok(())
}

pub(super) fn native_reboot_stopped(tx: &Transaction<'_>, command: &RunnerCommand) -> Result<bool> {
    if command.kind != CommandKind::RunAgent || command.state != CommandState::Accepted
        || command.agent_progress.as_ref().is_none_or(|p|
            !matches!(p.phase, AgentPhase::Starting | AgentPhase::Running))
        || command.agent_spec.as_ref().is_none_or(|s| s.read_only || s.continuation.is_some())
        || command.workspace_spec.as_ref().is_some_and(|s| s.target_git.is_some())
        || super::termination::binding(tx, command.id)?.is_some()
        || super::termination::active(tx, command.stream_id)?
        || super::controls::load(tx, command.stream_id)?.is_some_and(|c| c.state != PauseState::Released)
    {
        return Ok(false);
    }
    let scheduled: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM scheduler_runs WHERE agent_id=?1)",
        [command.id.to_string()], |row| row.get(0))?;
    if !scheduled {
        return Ok(false);
    }
    let Some(current) = native_boot(tx, command.runner_id)? else { return Ok(false); };
    let raw: Option<String> = tx.query_row("SELECT payload FROM events
        WHERE aggregate_id=?1 AND kind='agent.native_start_boot' ORDER BY sequence LIMIT 1",
        [command.id.to_string()], |row| row.get(0)).optional()?;
    let Some(raw) = raw else { return Ok(false); };
    let original: NativeRunnerBoot = serde_json::from_str(&raw)?;
    Ok(original.machine == current.machine && original.boot != current.boot)
}

impl Store {
    pub(crate) fn observe_native_runner_boot(&self, runner: RunnerId, boot: Option<NativeRunnerBoot>) -> Result<()> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        if let Some(boot) = boot {
            db.execute("INSERT INTO native_runner_boots VALUES(?1,?2)
                ON CONFLICT(runner_id) DO UPDATE SET data=excluded.data",
                params![runner.to_string(), serde_json::to_string(&boot)?])?;
        } else {
            db.execute("DELETE FROM native_runner_boots WHERE runner_id=?1", [runner.to_string()])?;
        }
        self.wake_scheduler();
        Ok(())
    }

    pub fn require_checkpoint_support(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        supported: bool,
        automatic: bool,
    ) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let command = super::work::command(&tx, id, runner, epoch)?;
        if native_reboot_stopped(&tx, &command)? {
            let owner = super::work::stream(&tx, command.stream_id)?;
            let automatic_project: bool = tx.query_row("SELECT COALESCE(json_extract(data,'$.runner_mapping')='automatic',0)
                FROM active_projects WHERE id=?1", [owner.project_id.to_string()], |row| row.get(0))?;
            if automatic_project && automatic_baseline(&tx, &owner)?.is_some() {
                return Err(StoreError::WorkConflict);
            }
        }
        if !supported && command.requires_checkpoint() {
            return Err(StoreError::WorkConflict);
        }
        if !automatic && ((command.kind == CommandKind::Checkpoint
            && load(&tx, id)?.is_some_and(|spec| spec.committed_only))
            || command.workspace_spec.as_ref().and_then(|spec| spec.setup.as_ref())
                .is_some_and(|setup| matches!(setup.repository_source.as_str(), CHECKPOINT_REPOSITORY | CHECKPOINT_RETURN)))
        {
            return Err(StoreError::WorkConflict);
        }
        if !automatic && command.kind == CommandKind::RunAgent {
            let owner = super::work::stream(&tx, command.stream_id)?;
            if automatic_baseline(&tx, &owner)?.is_some() {
                return Err(StoreError::WorkConflict);
            }
        }
        Ok(())
    }
    pub fn upload_checkpoint_bundle(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        bytes: Vec<u8>,
    ) -> Result<BundleReceipt> {
        if bytes.is_empty() || bytes.len() > CHECKPOINT_BUNDLE_LIMIT {
            return Err(StoreError::Invalid(
                "Checkpoint bundle exceeds 64 MiB limit",
            ));
        }
        let receipt = BundleReceipt {
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes: bytes.len() as u64,
        };
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let data: Option<String> = tx
            .query_row(
                "SELECT data FROM commands WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let c: RunnerCommand = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
        if c.runner_id != runner
            || c.ownership_epoch != epoch
            || c.kind != CommandKind::Checkpoint
            || load(&tx, id)?.is_none()
        {
            return Err(StoreError::WorkConflict);
        }
        let previous: Option<(String, u64)> = tx
            .query_row(
                "SELECT sha256,size FROM checkpoint_bundles WHERE command_id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((sha, size)) = previous {
            return if sha == receipt.sha256 && size == receipt.bytes {
                Ok(receipt)
            } else {
                Err(StoreError::WorkConflict)
            };
        }
        if c.state != CommandState::Accepted {
            return Err(StoreError::WorkConflict);
        }
        // Existing matching bytes are a read-only receipt replay even after
        // handoff. New writes still require the current ownership epoch.
        super::work::command(&tx, id, runner, epoch)?;
        tx.execute(
            "INSERT INTO checkpoint_bundles(command_id,sha256,size,content) VALUES(?1,?2,?3,?4)",
            params![id.to_string(), receipt.sha256, receipt.bytes, bytes],
        )?;
        event(&tx, "checkpoint.bundle", id.to_string(), &receipt)?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn download_checkpoint_bundle(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
    ) -> Result<Vec<u8>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let c = super::work::command(&tx, id, runner, epoch)?;
        if !matches!(
            c.kind,
            CommandKind::PrepareWorkspace | CommandKind::ReviewCheckpoint
        ) || c.state != CommandState::Accepted
        {
            return Err(StoreError::WorkConflict);
        }
        let import = if c.kind == CommandKind::ReviewCheckpoint {
            let ReviewJob::Review { checkpoint, .. } = super::reviews::load(&tx, id)? else {
                return Err(StoreError::WorkConflict);
            };
            checkpoint
        } else {
            c.workspace_spec
                .and_then(|s| s.checkpoint)
                .ok_or(StoreError::WorkConflict)?
        };
        let bytes: Vec<u8> = tx.query_row(
            "SELECT content FROM checkpoint_bundles WHERE command_id=?1 AND sha256=?2 AND size=?3",
            params![
                import.command_id.to_string(),
                import.bundle_sha256,
                import.bundle_bytes
            ],
            |r| r.get(0),
        )?;
        Ok(bytes)
    }
    pub fn handoff(&self, id: StreamId, body: HandoffStream) -> Result<Workstream> {
        self.mutate(
            body.request_id,
            &format!("stream.handoff/{id}"),
            &body,
            |tx| {
                line(&body.reason, 512)?;
                let mut owner = super::work::stream(tx, id)?;
                if owner.revision != body.expected_revision {
                    return Err(StoreError::RevisionConflict(owner.revision));
                }
                if body.target_runner == owner.runner_id
                    || super::draft_resets::retired(tx, id)?
                    || super::controls::load(tx, id)?
                        .is_some_and(|c| c.state != PauseState::Released)
                    || super::termination::active(tx, id)?
                {
                    return Err(StoreError::WorkConflict);
                }
                let fixed_runner: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM supervisor_policies WHERE stream_id=?1 UNION ALL SELECT 1 FROM directs WHERE stream_id=?1)",
                    [id.to_string()],
                    |r| r.get(0),
                )?;
                if fixed_runner {
                    return Err(StoreError::WorkConflict);
                }
                let data: Option<String> = tx
                    .query_row(
                        "SELECT data FROM commands WHERE stream_id=?1 ORDER BY rowid DESC LIMIT 1",
                        [id.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?;
                let c: RunnerCommand =
                    serde_json::from_str(&data.ok_or(StoreError::WorkConflict)?)?;
                if c.id != body.checkpoint_command
                    || c.kind != CommandKind::Checkpoint
                    || c.state != CommandState::Succeeded
                    || c.runner_id != owner.runner_id
                    || c.ownership_epoch != owner.ownership_epoch
                {
                    return Err(StoreError::WorkConflict);
                }
                let Some(CommandOutput::Checkpoint { checkpoint }) = c.result else {
                    return Err(StoreError::WorkConflict);
                };
                verify_bundle(tx, c.id, &checkpoint)?;
                owner.checkpoint = Some(CheckpointImport {
                    command_id: c.id,
                    commit: checkpoint.commit,
                    bundle_sha256: checkpoint.bundle_sha256,
                    bundle_bytes: checkpoint.bundle_bytes,
                });
                owner.runner_id = body.target_runner;
                owner.ownership_epoch = owner
                    .ownership_epoch
                    .checked_add(1)
                    .ok_or(StoreError::Internal)?;
                owner.branch = format!("factory/{id}-epoch-{}", owner.ownership_epoch);
                super::work::save_stream(tx, &mut owner, StreamState::Idle)?;
                event(
                    tx,
                    "stream.handoff",
                    id.to_string(),
                    &serde_json::json!({"request":body,"stream":owner}),
                )?;
                Ok(owner)
            },
        )
    }
    pub fn checkpoint(&self, id: CommandId) -> Result<Option<CheckpointSpec>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        load(&tx, id)
    }
    pub fn runner_checkpoint(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
    ) -> Result<CheckpointSpec> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let c = super::work::command(&tx, id, runner, epoch)?;
        if c.kind != CommandKind::Checkpoint || c.state != CommandState::Accepted {
            return Err(StoreError::WorkConflict);
        }
        load(&tx, id)?.ok_or(StoreError::NotFound)
    }
}
pub(super) fn verify_bundle(tx: &Transaction<'_>, id: CommandId, c: &GitCheckpoint) -> Result<()> {
    let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM checkpoint_bundles WHERE command_id=?1 AND sha256=?2 AND size=?3)",params![id.to_string(),c.bundle_sha256,c.bundle_bytes],|r|r.get(0))?;
    if !valid {
        return Err(StoreError::WorkConflict);
    }
    Ok(())
}
pub(super) fn validate(c: &GitCheckpoint) -> Result<()> {
    for hash in [&c.commit, &c.parent, &c.tree] {
        if !matches!(hash.len(), 40 | 64) || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StoreError::Invalid("Invalid checkpoint object ID"));
        }
    }
    if c.bundle_sha256.len() != 64
        || !c.bundle_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || c.bundle_bytes == 0
    {
        return Err(StoreError::Invalid("Invalid checkpoint bundle"));
    }
    for path in [&c.bundle_path, &c.manifest_path] {
        line(path, 4096)?;
        if !super::work::absolute_runner_path(path) {
            return Err(StoreError::Invalid("Absolute artifact path required"));
        }
    }
    Ok(())
}
