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
pub(super) fn create(tx: &Transaction<'_>, cmd: &RunnerCommand, owner: &Workstream) -> Result<()> {
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
        if !own_stop
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
    let spec=CheckpointSpec {command_id:cmd.id,source_commands:commands.iter().map(|c|c.id).collect(),stopped_runtimes:proofs,context:serde_json::json!({"project_id":owner.project_id,"stream_id":owner.id,"goal":owner.goal,"ticket_ids":owner.ticket_ids,"branch":owner.branch,"predecessor_checkpoint":owner.checkpoint,"profile":project.profile,"knowledge":knowledge.text,"archives_may_be_incomplete":!commands.is_empty()}).to_string()};
    tx.execute(
        "INSERT INTO checkpoints(command_id,data) VALUES(?1,?2)",
        params![cmd.id.to_string(), serde_json::to_string(&spec)?],
    )?;
    event(tx, "checkpoint.requested", cmd.id.to_string(), &spec)
}
impl Store {
    pub fn require_checkpoint_support(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        supported: bool,
    ) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        if !supported && super::work::command(&tx, id, runner, epoch)?.requires_checkpoint() {
            return Err(StoreError::WorkConflict);
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
                    || super::controls::load(tx, id)?
                        .is_some_and(|c| c.state != PauseState::Released)
                    || super::termination::active(tx, id)?
                {
                    return Err(StoreError::WorkConflict);
                }
                let supervised: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM supervisor_policies WHERE stream_id=?1)",
                    [id.to_string()],
                    |r| r.get(0),
                )?;
                if supervised {
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
        if !path.starts_with('/') {
            return Err(StoreError::Invalid("Absolute artifact path required"));
        }
    }
    Ok(())
}
