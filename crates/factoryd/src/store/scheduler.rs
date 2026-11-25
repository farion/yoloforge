use super::*;
use std::collections::{BTreeMap, BTreeSet};
fn tickets(tx: &Transaction<'_>, project: ProjectId) -> Result<BTreeMap<TicketId, Ticket>> {
    let mut stmt = tx.prepare("SELECT data FROM tickets WHERE project_id=?1 ORDER BY rowid")?;
    let rows = stmt.query_map([project.to_string()], |r| r.get::<_, String>(0))?;
    rows.map(|r| {
        let t: Ticket = serde_json::from_str(&r?)?;
        Ok((t.id, t))
    })
    .collect()
}
fn policy(tx: &Transaction<'_>, id: ProjectId) -> Result<Option<SchedulerPolicy>> {
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM scheduler_policies WHERE project_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    data.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}
fn save_policy(tx: &Transaction<'_>, p: &SchedulerPolicy) -> Result<()> {
    tx.execute("INSERT INTO scheduler_policies(project_id,data) VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET data=excluded.data",params![p.project_id.to_string(),serde_json::to_string(p)?])?;
    event(tx, "scheduler.policy", p.project_id.to_string(), p)
}
fn bind_project_runner(project: &mut Project, runner: RunnerId) {
    let binding = project.runner_bindings.entry(runner).or_insert_with(|| ProjectLocation {
        repository_path: String::new(),
        service_registration: String::new(),
    });
    project.setup = Some(ProjectSetup {
        runner_id: runner,
        repository_source: project.repository.clone(),
        repository_path: binding.repository_path.clone(),
        service_registration: binding.service_registration.clone(),
        harness: project.setup.as_ref().and_then(|setup| setup.harness.clone()),
    });
}

pub(super) fn map_project_runner(
    tx: &Transaction<'_>,
    project: &mut Project,
    mapping: ProjectRunnerMapping,
) -> Result<()> {
    if project.runner_mapping == Some(mapping) {
        return Ok(());
    }
    project.runner_mapping = Some(mapping);
    let runners = match mapping {
        ProjectRunnerMapping::Runner(runner) => {
            bind_project_runner(project, runner);
            vec![runner]
        }
        ProjectRunnerMapping::Automatic => Vec::new(),
    };
    if let Some(mut policy) = policy(tx, project.id)? {
        if policy.runner_ids != runners {
            policy.runner_ids = runners;
            policy.revision.0 += 1;
            save_policy(tx, &policy)?;
        }
    }
    Ok(())
}
pub(super) fn activate_prompts(tx: &Transaction<'_>, id: ProjectId) -> Result<()> {
    if let Some(mut p) = policy(tx, id)? {
        if !p.enabled {
            p.enabled = true;
            p.revision.0 += 1;
            save_policy(tx, &p)?;
        }
    }
    Ok(())
}
fn save_ticket(tx: &Transaction<'_>, t: &mut Ticket) -> Result<()> {
    t.revision.0 += 1;
    t.updated_unix_ms = super::super::unix_ms();
    tx.execute(
        "UPDATE tickets SET revision=?1,data=?2 WHERE id=?3",
        params![t.revision.0, serde_json::to_string(t)?, t.id.to_string()],
    )?;
    event(tx, "ticket.updated", t.id.to_string(), t)
}
fn done(t: &Ticket) -> bool {
    matches!(t.status, TicketState::Done | TicketState::Cancelled)
}

fn latest_command(tx: &Transaction<'_>, stream: StreamId) -> Result<RunnerCommand> {
    let data: String = tx.query_row(
        "SELECT data FROM commands WHERE stream_id=?1 ORDER BY rowid DESC LIMIT 1",
        [stream.to_string()], |row| row.get(0),
    )?;
    Ok(serde_json::from_str(&data)?)
}

fn checkpoint_runner_ready(tx: &Transaction<'_>, runner: RunnerId, since: u64) -> Result<bool> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM live_automatic_runners
        WHERE runner_id=?1 AND seen_ms>=?2 AND checkpoint=1)",
        params![runner.to_string(), since], |row| row.get(0))?)
}

fn failover_target(tx: &Transaction<'_>, source: RunnerId, ticket: &Ticket, since: u64) -> Result<Option<RunnerId>> {
    automatic_target(tx, source, ticket, since, false)
}

fn automatic_target(
    tx: &Transaction<'_>, source: RunnerId, ticket: &Ticket, since: u64, server_only: bool,
) -> Result<Option<RunnerId>> {
    let mcps = super::mcp::for_ticket(tx, ticket.id, ticket.project_id)?;
    if !super::mcp::allowed(tx, ticket.project_id, &mcps)? {
        return Ok(None);
    }
    let mut query = tx.prepare("SELECT runner_id FROM live_automatic_runners
        WHERE runner_id<>?1 AND seen_ms>=?2 AND checkpoint=1
          AND (?4=0 OR server_runner=1)
          AND NOT EXISTS(SELECT 1 FROM runner_restart_holds h
                         WHERE h.runner_id=live_automatic_runners.runner_id AND h.expires_ms>?3)
        ORDER BY server_runner DESC, runner_id")?;
    let rows = query.query_map(
        params![source.to_string(), since, super::super::unix_ms(), server_only],
        |row| row.get::<_, String>(0),
    )?;
    for id in rows {
        let id = id?.parse().map_err(|_| StoreError::Internal)?;
        if super::mcp::runner_ready(tx, id, &mcps)? {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

fn reassign_from_checkpoint(
    tx: &Transaction<'_>, owner: Workstream, ticket: &Ticket,
    checkpoint: &RunnerCommand, pending: Option<RunnerCommand>, since: u64,
) -> Result<bool> {
    reassign_checkpoint_to(tx, owner, ticket, checkpoint, pending, since, None)
}

fn reassign_checkpoint_to(
    tx: &Transaction<'_>, mut owner: Workstream, ticket: &Ticket, checkpoint: &RunnerCommand,
    pending: Option<RunnerCommand>, since: u64, preferred: Option<RunnerId>,
) -> Result<bool> {
    let import = match &checkpoint.result {
        Some(CommandOutput::Checkpoint { checkpoint: artifact })
            if checkpoint.state == CommandState::Succeeded
                && checkpoint.runner_id == owner.runner_id
                && checkpoint.ownership_epoch == owner.ownership_epoch =>
        {
            super::checkpoints::verify_bundle(tx, checkpoint.id, artifact)?;
            CheckpointImport {
                command_id: checkpoint.id, commit: artifact.commit.clone(),
                bundle_sha256: artifact.bundle_sha256.clone(), bundle_bytes: artifact.bundle_bytes,
            }
        }
        _ => return Ok(false),
    };
    if super::controls::load(tx, owner.id)?.is_some_and(|state| state.state != PauseState::Released)
        || super::termination::active(tx, owner.id)?
    {
        return Ok(false);
    }
    let target = match preferred {
        Some(target) => target,
        None => {
            let Some(target) = failover_target(tx, owner.runner_id, ticket, since)? else { return Ok(false); };
            target
        }
    };
    if let Some(mut old) = pending {
        // Start authorization and this transfer serialize in the same store.
        // An acknowledgement reserves a job; it does not authorize OpenCode writes.
        let native_stopped = super::checkpoints::native_reboot_stopped(tx, &old)?;
        if !matches!(old.state, CommandState::Pending | CommandState::Accepted)
            || old.kind != CommandKind::RunAgent
            || (old.agent_progress.is_some() && !native_stopped)
            || super::termination::binding(tx, old.id)?.is_some()
        {
            return Ok(false);
        }
        old.state = CommandState::Failed;
        super::work::save_command(tx, &old, "failed")?;
        if native_stopped {
            super::questions::close(tx, old.id)?;
            super::compaction::terminate(tx, old.id)?;
            tx.execute("UPDATE scheduler_runs SET prompt=prompt || ?1 WHERE ticket_id=?2", params![
                "\n\nRecovery: the previous runner rebooted. This run uses the confirmed checkpoint from before the previous execution. \
                  Verify potentially completed external actions before repeating them.",
                ticket.id.to_string(),
            ])?;
            event(tx, "scheduler.native_reboot_recovery", old.id.to_string(), &owner.id)?;
        }
    }
    owner.runner_id = target;
    owner.ownership_epoch = owner.ownership_epoch.checked_add(1).ok_or(StoreError::Internal)?;
    owner.branch = format!("factory/{}-epoch-{}", owner.id, owner.ownership_epoch);
    owner.checkpoint = Some(import);
    super::work::save_stream(tx, &mut owner, StreamState::Idle)?;
    let prepare = Store::enqueue_in(tx, owner.id, &EnqueueCommand {
        request_id: RequestId::new(), expected_revision: owner.revision,
        kind: CommandKind::PrepareWorkspace, prompt: None,
    }, None, false)?;
    tx.execute("UPDATE scheduler_runs SET prepare_id=?1, agent_id=NULL WHERE ticket_id=?2",
        params![prepare.id.to_string(), ticket.id.to_string()])?;
    let mut ticket = ticket.clone();
    ticket.execution_command = Some(prepare.id);
    ticket.routing_note = if preferred.is_some() {
        "Auto: Der Server-Runner übernimmt den bestätigten Projektstand vor dem Start."
    } else {
        "Auto: Projektstand wird aus einem bestätigten Checkpoint auf dem neuen Runner vorbereitet."
    }.into();
    save_ticket(tx, &mut ticket)?;
    event(tx, "scheduler.runner_failover", owner.id.to_string(), &owner)?;
    Ok(true)
}

fn move_automatic_work(tx: &Transaction<'_>, project: ProjectId, since: u64) -> Result<()> {
    let mut stmt = tx.prepare("SELECT c.data,t.data FROM scheduler_runs r
        JOIN tickets t ON t.id=r.ticket_id JOIN commands c ON c.id=r.agent_id
        WHERE t.project_id=?1
          AND (c.state IN ('pending','accepted') OR
            (c.state='failed' AND EXISTS(SELECT 1 FROM runtime_terminations rt
              WHERE rt.command_id=c.id AND rt.state='stopped'
                AND json_extract(rt.data,'$.automatic_failover')=1)))
          AND json_extract(c.data,'$.kind')='run_agent'")?;
    let rows = stmt.query_map([project.to_string()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (raw_command, raw_ticket) in rows {
        let command: RunnerCommand = serde_json::from_str(&raw_command)?;
        let ticket: Ticket = serde_json::from_str(&raw_ticket)?;
        let owner = super::work::stream(tx, command.stream_id)?;
        if ticket.status != TicketState::InProgress || latest_command(tx, owner.id)?.id != command.id
            || ticket.execution_command != Some(command.id)
            || command.runner_id != owner.runner_id
            || command.ownership_epoch != owner.ownership_epoch
            || command.agent_spec.as_ref().is_none_or(|spec| spec.read_only || spec.continuation.is_some())
            || command.workspace_spec.as_ref().is_some_and(|spec| spec.target_git.is_some())
        {
            continue;
        }
        let Some(checkpoint) = super::checkpoints::automatic_baseline(tx, &owner)? else { continue; };
        if command.state == CommandState::Failed {
            if super::termination::load(tx, command.id)?.is_some_and(|stop|
                stop.automatic_failover && stop.state == TerminationState::Stopped && stop.proof.is_some())
                && reassign_from_checkpoint(tx, owner, &ticket, &checkpoint, None, since)?
            {
                tx.execute("UPDATE scheduler_runs SET prompt=prompt || ?1 WHERE ticket_id=?2", params![
                    "\n\nRecovery: the previous isolated execution was stopped. This workspace was restored \
                     from the checkpoint taken before that execution. Verify any potentially completed \
                     external actions before repeating them. Do not assume they were rolled back.",
                    ticket.id.to_string(),
                ])?;
            }
            continue;
        }
        if super::checkpoints::native_reboot_stopped(tx, &command)? {
            reassign_from_checkpoint(tx, owner, &ticket, &checkpoint, Some(command), since)?;
            continue;
        }
        if checkpoint_runner_ready(tx, owner.runner_id, since)? {
            continue;
        }
        if command.state == CommandState::Accepted {
            if let Some(binding) = super::termination::binding(tx, command.id)? {
                let missing_since = super::super::unix_ms().saturating_sub(UNREACHABLE_AFTER.as_millis() as u64);
                // A missing registration after factoryd restarts isn't evidence
                // of loss. Only act on an observed contact that has gone stale.
                let contact_expired: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM automatic_runner_contacts
                    WHERE runner_id=?1 AND seen_ms<=?2)",
                    params![owner.runner_id.to_string(), missing_since], |row| row.get(0))?;
                if contact_expired
                    && command.agent_progress.as_ref().is_some_and(|p|
                        matches!(p.phase, AgentPhase::Starting | AgentPhase::Running))
                    && super::termination::load(tx, command.id)?.is_none()
                    && super::controls::load(tx, owner.id)?.is_none_or(|control| control.state == PauseState::Released)
                    && failover_target(tx, owner.runner_id, &ticket, since)?.is_some()
                {
                    let CommandOutput::Checkpoint { checkpoint: artifact } =
                        checkpoint.result.as_ref().ok_or(StoreError::Internal)? else { return Err(StoreError::Internal); };
                    super::checkpoints::verify_bundle(tx, checkpoint.id, artifact)?;
                    Store::request_termination_with_failover(tx, command.id, &RequestTermination {
                        request_id: RequestId::new(), expected_stream_revision: owner.revision,
                        runtime: binding.runtime, previous_request: None,
                        reason: "Auto: confirm old runtime stopped before restarting from checkpoint".into(),
                    }, true)?;
                    let mut ticket = ticket;
                    super::routing::note(tx, &mut ticket, "Auto wartet auf den Stoppnachweis der bisherigen Laufzeit.")?;
                }
                continue;
            }
        }
        reassign_from_checkpoint(tx, owner, &ticket, &checkpoint, Some(command), since)?;
    }
    Ok(())
}

fn export_portable_results(tx: &Transaction<'_>, project: ProjectId, since: u64) -> Result<()> {
    let mut stmt = tx.prepare("SELECT c.data FROM scheduler_runs r
        JOIN tickets t ON t.id=r.ticket_id JOIN commands c ON c.id=r.agent_id
        WHERE t.project_id=?1 AND c.state='succeeded'
          AND json_extract(c.data,'$.kind')='run_agent'
          AND json_extract(c.data,'$.workspace_spec.setup.repository_source') IN (?2,?3)
          AND c.rowid=(SELECT MAX(latest.rowid) FROM commands latest WHERE latest.stream_id=c.stream_id)
        ORDER BY c.rowid")?;
    let rows = stmt.query_map(params![project.to_string(), CHECKPOINT_REPOSITORY, "factory:managed"], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for raw in rows {
        let completed: RunnerCommand = serde_json::from_str(&raw)?;
        let owner = super::work::stream(tx, completed.stream_id)?;
        if !checkpoint_runner_ready(tx, owner.runner_id, since)?
            || completed.runner_id != owner.runner_id || completed.ownership_epoch != owner.ownership_epoch
        {
            continue;
        }
        if let Some(origin) = super::checkpoints::portable_result_origin(tx, &owner, &completed)? {
            tx.execute_batch("SAVEPOINT export_project_result")?;
            match Store::enqueue_portable_result(tx, &owner, origin) {
                Ok(_) => {}
                Err(StoreError::WorkConflict | StoreError::Invalid(_) | StoreError::RevisionConflict(_)) =>
                    tx.execute_batch("ROLLBACK TO export_project_result")?,
                Err(error) => return Err(error),
            }
            tx.execute_batch("RELEASE export_project_result")?;
        }
    }
    Ok(())
}

fn return_portable_result(tx: &Transaction<'_>, project_id: ProjectId, since: u64) -> Result<()> {
    let raw: String = tx.query_row("SELECT data FROM active_projects WHERE id=?1",
        [project_id.to_string()], |row| row.get(0))?;
    let project: Project = serde_json::from_str(&raw)?;
    let Some(source) = &project.setup else { return Ok(()); };
    if !checkpoint_runner_ready(tx, source.runner_id, since)? {
        return Ok(());
    }
    let Some(seed) = super::checkpoints::offline_project_seed(tx, &project, source)? else { return Ok(()); };
    if !super::checkpoints::load(tx, seed.command_id)?.is_some_and(|spec| spec.project_result.is_some()) {
        return Ok(());
    }
    let path = if source.repository_path.is_empty() { &source.repository_source } else { &source.repository_path };
    if !super::work::absolute_runner_path(path) {
        return Ok(());
    }
    let now = super::super::unix_ms();
    let mut stale = tx.prepare("SELECT c.data FROM commands c JOIN streams s ON s.id=c.stream_id
        WHERE s.project_id=?1 AND c.runner_id=?2 AND c.state='pending'
          AND json_extract(c.data,'$.workspace_spec.setup.repository_source')=?3
          AND json_extract(c.data,'$.workspace_spec.checkpoint.command_id')<>?4")?;
    let obsolete = stale.query_map(params![project.id.to_string(), source.runner_id.to_string(),
        CHECKPOINT_RETURN, seed.command_id.to_string()], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stale);
    for raw in obsolete {
        let mut command: RunnerCommand = serde_json::from_str(&raw)?;
        command.state = CommandState::Failed;
        super::work::save_command(tx, &command, "failed")?;
        let mut stream = super::work::stream(tx, command.stream_id)?;
        super::work::save_stream(tx, &mut stream, StreamState::Blocked)?;
    }
    let already_scheduled: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM commands c JOIN streams s ON s.id=c.stream_id
           WHERE s.project_id=?1 AND c.runner_id=?2
             AND json_extract(c.data,'$.workspace_spec.setup.repository_source')=?3
             AND (c.state IN ('pending','accepted') OR
                  (json_extract(c.data,'$.workspace_spec.checkpoint.command_id')=?4
                   AND (c.state='succeeded' OR EXISTS(SELECT 1 FROM events e
                      WHERE e.aggregate_id=c.id AND e.kind='command.updated' AND e.created_unix_ms>?5)))))",
        params![project.id.to_string(), source.runner_id.to_string(), CHECKPOINT_RETURN,
            seed.command_id.to_string(), now.saturating_sub(300_000)], |row| row.get(0),
    )?;
    if already_scheduled {
        return Ok(());
    }
    let mut stream = Store::create_stream_in(tx, &CreateStream {
        request_id: RequestId::new(), project_id, runner_id: source.runner_id,
        goal: "Projektstand lokal übernehmen".into(), ticket_ids: vec![],
    })?;
    super::checkpoints::seed_stream(tx, &mut stream, Some(&seed))?;
    let command = Store::enqueue_portable_return(tx, &stream, path)?;
    event(tx, "scheduler.project_return", project.id.to_string(), &command)
}
fn return_project_mirrors(tx: &Transaction<'_>, project_id: ProjectId, since: u64) -> Result<()> {
    let raw: String = tx.query_row("SELECT data FROM active_projects WHERE id=?1",
        [project_id.to_string()], |row| row.get(0))?;
    let project: Project = serde_json::from_str(&raw)?;
    if project.repository != "factory:managed" || project.mirrors.is_empty() {
        return Ok(());
    }
    let Some(source) = &project.setup else { return Ok(()); };
    let Some(seed) = super::checkpoints::offline_project_seed(tx, &project, source)? else { return Ok(()); };
    if !super::checkpoints::load(tx, seed.command_id)?.is_some_and(|spec| spec.project_result.is_some()) {
        return Ok(());
    }
    let now = super::super::unix_ms();
    for (runner, mirror) in &project.mirrors {
        let ready: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM live_automatic_runners
             WHERE runner_id=?1 AND seen_ms>=?2 AND local_source=1 AND checkpoint=1)",
            params![runner.to_string(), since], |row| row.get(0),
        )?;
        if !ready || !super::work::absolute_runner_path(&mirror.path) {
            continue;
        }
        let mut stale = tx.prepare("SELECT c.data FROM commands c JOIN streams s ON s.id=c.stream_id
            WHERE s.project_id=?1 AND c.runner_id=?2 AND c.state='pending'
              AND json_extract(c.data,'$.workspace_spec.setup.repository_source')=?3
              AND json_extract(c.data,'$.workspace_spec.setup.repository_path')=?4
              AND json_extract(c.data,'$.workspace_spec.checkpoint.command_id')<>?5")?;
        let obsolete = stale.query_map(params![project.id.to_string(), runner.to_string(),
            CHECKPOINT_RETURN, mirror.path, seed.command_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stale);
        for raw in obsolete {
            let mut command: RunnerCommand = serde_json::from_str(&raw)?;
            command.state = CommandState::Failed;
            super::work::save_command(tx, &command, "failed")?;
            let mut stream = super::work::stream(tx, command.stream_id)?;
            super::work::save_stream(tx, &mut stream, StreamState::Blocked)?;
        }
        let scheduled: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands c JOIN streams s ON s.id=c.stream_id
             WHERE s.project_id=?1 AND c.runner_id=?2
               AND json_extract(c.data,'$.workspace_spec.setup.repository_source')=?3
               AND json_extract(c.data,'$.workspace_spec.setup.repository_path')=?4
               AND json_extract(c.data,'$.workspace_spec.checkpoint.command_id')=?5
               AND (c.state IN ('pending','accepted','succeeded')
                    OR (c.state='failed' AND json_extract(c.data,'$.created_unix_ms')>?6)))",
            params![project.id.to_string(), runner.to_string(), CHECKPOINT_RETURN,
                mirror.path, seed.command_id.to_string(), now.saturating_sub(300_000)],
            |row| row.get(0),
        )?;
        if scheduled {
            continue;
        }
        let mut stream = Store::create_stream_in(tx, &CreateStream {
            request_id: RequestId::new(), project_id, runner_id: *runner,
            goal: "Factory-Projekt lokal spiegeln".into(), ticket_ids: vec![],
        })?;
        super::checkpoints::seed_stream(tx, &mut stream, Some(&seed))?;
        let command = Store::enqueue_portable_return(tx, &stream, &mirror.path)?;
        event(tx, "scheduler.project_mirror", project.id.to_string(), &command)?;
    }
    Ok(())
}
pub(super) fn set_limit(tx: &Transaction<'_>, id: ProjectId, limit: u8) -> Result<()> {
    if let Some(mut p) = policy(tx, id)? {
        p.max_parallel = limit;
        p.revision.0 += 1;
        save_policy(tx, &p)?;
    }
    Ok(())
}

fn is_local_repository(repository: &str) -> bool {
    repository.starts_with('/')
        || repository.starts_with("\\\\")
        || matches!(repository.as_bytes(), [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic())
}

fn refresh_automatic_runner(tx: &Transaction<'_>, policy: &mut SchedulerPolicy, ticket: &Ticket) -> Result<()> {
    let data: String = tx.query_row(
        "SELECT data FROM active_projects WHERE id=?1",
        [policy.project_id.to_string()], |row| row.get(0),
    )?;
    let mut project: Project = serde_json::from_str(&data)?;
    match project.runner_mapping {
        Some(ProjectRunnerMapping::Runner(_)) => return Ok(()),
        // Legacy projects displayed as Automatic retained their initial host in setup.
        None if !project.setup.as_ref().is_some_and(|setup| policy.runner_ids == [setup.runner_id]) => return Ok(()),
        _ => {}
    }
    let now = super::super::unix_ms().saturating_sub(30_000);
    let mut query = tx.prepare(
        "SELECT runner_id,local_source FROM live_automatic_runners WHERE seen_ms >= ?1 ORDER BY runner_id",
    )?;
    let candidates = query.query_map([now], |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let local_repository = is_local_repository(&project.repository);
    let mcps = super::mcp::for_ticket(tx, ticket.id, project.id)?;
    if !super::mcp::allowed(tx, project.id, &mcps)? {
        return Ok(());
    }
    let mut replacement = None;
    for (id, local_source) in candidates {
        let id = id.parse().map_err(|_| StoreError::Internal)?;
        if (local_repository && !local_source && !project.runner_bindings.contains_key(&id))
            || !super::mcp::runner_ready(tx, id, &mcps)?
        {
            continue;
        }
        if policy.runner_ids.contains(&id) {
            return Ok(());
        }
        if replacement.is_none() {
            replacement = Some(id);
        }
    }
    let Some(id) = replacement else { return Ok(()); };
    bind_project_runner(&mut project, id);
    project.runner_mapping = Some(ProjectRunnerMapping::Automatic);
    project.revision.0 += 1;
    tx.execute("UPDATE projects SET data=?1 WHERE id=?2",
        params![serde_json::to_string(&project)?, project.id.to_string()])?;
    event(tx, "project.automatic_setup", project.id.to_string(), &project)?;
    policy.runner_ids = vec![id];
    policy.revision.0 += 1;
    save_policy(tx, policy)
}

fn portable_source(tx: &Transaction<'_>, project: ProjectId, since: u64) -> Result<Option<(Project, ProjectSetup, bool)>> {
    let data: String = tx.query_row("SELECT data FROM active_projects WHERE id=?1",
        [project.to_string()], |row| row.get(0))?;
    let project: Project = serde_json::from_str(&data)?;
    let Some(source) = project.setup.clone().filter(|source| {
        source.repository_source == project.repository && super::work::absolute_runner_path(&project.repository)
    }) else { return Ok(None); };
    let offline: bool = tx.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM live_automatic_runners WHERE runner_id=?1 AND seen_ms>=?2)
           AND (EXISTS(SELECT 1 FROM automatic_runner_contacts WHERE runner_id=?1)
                OR EXISTS(SELECT 1 FROM commands c JOIN streams s ON s.id=c.stream_id
                          JOIN checkpoints cp ON cp.command_id=c.id
                          WHERE s.project_id=?3 AND c.runner_id=?1
                          AND json_extract(cp.data,'$.committed_only')=1))",
        params![source.runner_id.to_string(), since, project.id.to_string()], |row| row.get(0),
    )?;
    if offline || super::checkpoints::has_unreturned_portable_result(tx, &project, &source)? {
        Ok(Some((project, source, offline)))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod automatic_runner_tests {
    use super::*;

    #[test]
    fn automatic_work_moves_until_the_runner_has_obtained_start_authorization() {
        for (accepted, started, contained, rebooted, server_first) in [
            (false, false, false, false, false), (true, false, false, false, false),
            (true, true, false, false, false), (true, true, true, false, false),
            (true, true, false, true, false), (false, false, false, false, true),
        ] {
            let store = Store::open(std::path::Path::new(":memory:")).unwrap();
            let source = RunnerId::new();
            let target = RunnerId::new();
            let project = store.create_project(CreateProject {
                request_id: RequestId::new(), name: "Failover".into(), key: None, logo: None,
                repository: "/desktop/private/checkout".into(), base_branch: "main".into(),
                runner_mapping: Some(ProjectRunnerMapping::Automatic),
            }).unwrap();
            store.update_profile(project.id, UpdateProfile {
                request_id: RequestId::new(), expected_revision: project.revision,
                profile: ExecutionProfile { provider_id: "test".into(), model_id: "test".into(), ..Default::default() },
            }).unwrap();
            let ticket = store.save_task(SaveTask {
                request_id: RequestId::new(), id: None, expected_revision: Revision(0),
                project_id: project.id, requested_session: None, prompt: "Continue work".into(), images: vec![],
            }).unwrap();
            store.observe_automatic_runner(source, true, true, true).unwrap();
            store.observe_automatic_runner_location(target, true, false, true, server_first).unwrap();
            if server_first {
                store.autostart_runner_at(target, true).unwrap();
                assert!(store.scheduler_policy(project.id).unwrap().is_none(),
                    "The server runner cannot claim the desktop checkout path");
            }
            store.autostart_runner(source).unwrap();
            store.scheduler_tick().unwrap();
            let [prepare]: [RunnerCommand; 1] = store.pending(source).unwrap().try_into().unwrap();
            assert_eq!(prepare.kind, CommandKind::PrepareWorkspace);
            assert!(prepare.workspace_spec.as_ref().unwrap().setup.is_some());
            assert_eq!(store.snapshot().unwrap().projects[0].runner_mapping, Some(ProjectRunnerMapping::Automatic));
            {
                let mut db = store.0.lock().unwrap();
                let tx = db.transaction().unwrap();
                assert!(checkpoint_runner_ready(&tx, source, 0).unwrap());
            }
            let ack = |c: &RunnerCommand| AckCommand { request_id: RequestId::new(), ownership_epoch: c.ownership_epoch };
            let complete = |c: &RunnerCommand, result| {
                store.complete(c.runner_id, c.id, CommandResult {
                    request_id: RequestId::new(), ownership_epoch: c.ownership_epoch, result,
                }).unwrap();
            };
            store.acknowledge(source, prepare.id, ack(&prepare)).unwrap();
            complete(&prepare, CommandOutput::Workspace { workspace: Workspace {
                path: "/source/worktree".into(), branch: prepare.workspace_spec.as_ref().unwrap().branch.clone(),
                base_commit: "a".repeat(40),
            }});
            store.scheduler_tick().unwrap();
            let [routing]: [RunnerCommand; 1] = store.pending(source).unwrap().try_into().unwrap();
            assert_eq!(routing.agent_spec.as_ref().unwrap().role, "orchestrator");
            store.acknowledge(source, routing.id, ack(&routing)).unwrap();
            complete(&routing, CommandOutput::Agent { agent: AgentProgress {
                session_id: routing.session_id(), message_id: agent_message_id(routing.id),
                phase: AgentPhase::Succeeded, summary: r#"{"action":"new","reason":"New workspace"}"#.into(),
            }});
            store.scheduler_tick().unwrap();
            let [prepare]: [RunnerCommand; 1] = store.pending(source).unwrap().try_into().unwrap();
            assert_eq!(prepare.kind, CommandKind::PrepareWorkspace);
            store.acknowledge(source, prepare.id, ack(&prepare)).unwrap();
            complete(&prepare, CommandOutput::Workspace { workspace: Workspace {
                path: "/source/implementation".into(), branch: prepare.workspace_spec.as_ref().unwrap().branch.clone(),
                base_commit: "a".repeat(40),
            }});
            store.scheduler_tick().unwrap();
            let [checkpoint]: [RunnerCommand; 1] = store.pending(source).unwrap().try_into().unwrap();
            assert_eq!(checkpoint.kind, CommandKind::Checkpoint);
            assert!(store.checkpoint(checkpoint.id).unwrap().unwrap().committed_only);
            assert!(store.require_checkpoint_support(source, checkpoint.id, 1, true, false).is_err());
            store.acknowledge(source, checkpoint.id, ack(&checkpoint)).unwrap();
            let bundle = b"fixture bundle".to_vec();
            let receipt = store.upload_checkpoint_bundle(source, checkpoint.id, 1, bundle.clone()).unwrap();
            complete(&checkpoint, CommandOutput::Checkpoint { checkpoint: GitCheckpoint {
                commit: "b".repeat(40), parent: "a".repeat(40), tree: "c".repeat(40),
                bundle_path: "/source/file.bundle".into(), manifest_path: "/source/manifest.json".into(),
                bundle_sha256: receipt.sha256, bundle_bytes: receipt.bytes,
            }});
            store.scheduler_tick().unwrap();
            if server_first {
                assert!(store.pending(source).unwrap().is_empty());
                let [remote_prepare]: [RunnerCommand; 1] = store.pending(target).unwrap().try_into().unwrap();
                let spec = remote_prepare.workspace_spec.as_ref().unwrap();
                assert_eq!(spec.checkpoint.as_ref().unwrap().command_id, checkpoint.id);
                assert_eq!(spec.setup.as_ref().unwrap().repository_source, CHECKPOINT_REPOSITORY);
                store.acknowledge(target, remote_prepare.id, ack(&remote_prepare)).unwrap();
                complete(&remote_prepare, CommandOutput::Workspace { workspace: Workspace {
                    path: "/target/worktree".into(), branch: spec.branch.clone(), base_commit: "b".repeat(40),
                }});
                store.scheduler_tick().unwrap();
                let [backup]: [RunnerCommand; 1] = store.pending(target).unwrap().try_into().unwrap();
                assert_eq!(backup.kind, CommandKind::Checkpoint);
                store.acknowledge(target, backup.id, ack(&backup)).unwrap();
                complete(&backup, CommandOutput::CheckpointError { checkpoint_error: "Fixture".into() });
                store.scheduler_tick().unwrap();
                let [remote]: [RunnerCommand; 1] = store.pending(target).unwrap().try_into().unwrap();
                assert_eq!(remote.kind, CommandKind::RunAgent);
                store.acknowledge(target, remote.id, ack(&remote)).unwrap();
                store.observe_automatic_runner(source, false, false, true).unwrap();
                store.scheduler_tick().unwrap();
                assert!(store.pending(source).unwrap().is_empty());
                assert_eq!(store.pending(target).unwrap()[0].id, remote.id);
                complete(&remote, CommandOutput::Agent { agent: AgentProgress {
                    session_id: remote.session_id(), message_id: agent_message_id(remote.id),
                    phase: AgentPhase::Succeeded, summary: "Finished with the desktop offline".into(),
                }});
                store.scheduler_tick().unwrap();
                assert!(store.pending(target).unwrap().iter().any(|c| c.kind == CommandKind::Checkpoint
                    && store.checkpoint(c.id).unwrap().unwrap().project_result.is_some()));
                continue;
            }
            let [agent]: [RunnerCommand; 1] = store.pending(source).unwrap().try_into().unwrap();
            assert_eq!(agent.kind, CommandKind::RunAgent);
            if accepted {
                store.acknowledge(source, agent.id, ack(&agent)).unwrap();
            }
            let start = || ObserveAgent {
                ownership_epoch: 1, progress: AgentProgress {
                    session_id: agent.session_id(), message_id: agent_message_id(agent.id),
                    phase: AgentPhase::Starting, summary: AGENT_START_GRANT.into(),
                },
            };
            let boot = |machine: &str, boot: &str| NativeRunnerBoot { machine: machine.repeat(64), boot: boot.repeat(64) };
            if rebooted {
                store.observe_native_runner_boot(source, Some(boot("a", "1"))).unwrap();
            }
            if started {
                store.observe_agent(source, agent.id, start()).unwrap();
            }
            let binding = RuntimeBinding {
                command_id: agent.id, stream_id: agent.stream_id, runner_id: source,
                ownership_epoch: 1, daemon_id: "test-engine".into(),
                runtime: RuntimeRef {
                    container_id: "a".repeat(64), started_at: "2026-11-21T00:00:00Z".into(),
                },
                endpoint: "http://127.0.0.1:4096".into(), workspace: "/source/implementation".into(),
            };
            if contained {
                store.report_runtime_binding(source, agent.id, binding.clone()).unwrap();
            }
            store.observe_automatic_runner(source, false, false, true).unwrap();
            store.scheduler_tick().unwrap();
            if started {
                assert!(store.pending(target).unwrap().is_empty());
                assert!(store.termination(agent.id).unwrap().is_none());
                store.0.lock().unwrap().execute(
                    "UPDATE automatic_runner_contacts SET seen_ms=0 WHERE runner_id=?1",
                    [source.to_string()],
                ).unwrap();
                store.scheduler_tick().unwrap();
                assert!(store.pending(target).unwrap().is_empty());
                if !contained {
                    assert!(store.termination(agent.id).unwrap().is_none());
                    if !rebooted {
                        continue;
                    }
                    store.observe_native_runner_boot(source, Some(boot("b", "2"))).unwrap();
                    store.scheduler_tick().unwrap();
                    assert!(store.pending(target).unwrap().is_empty());
                    store.observe_native_runner_boot(source, Some(boot("a", "1"))).unwrap();
                    store.scheduler_tick().unwrap();
                    assert!(store.pending(target).unwrap().is_empty());
                    store.observe_native_runner_boot(source, Some(boot("a", "2"))).unwrap();
                    store.observe_automatic_runner(source, true, true, true).unwrap();
                    assert!(store.require_checkpoint_support(source, agent.id, 1, true, true).is_err());
                    store.scheduler_tick().unwrap();
                } else {
                    let stop = store.termination(agent.id).unwrap().unwrap();
                    assert!(stop.automatic_failover);
                    assert_eq!(stop.state, TerminationState::Requested);
                    let mut proof = ReportTermination {
                        request_id: stop.request_id, ownership_epoch: 1, state: TerminationState::Stopped,
                        proof: Some(RemovalProof {
                            runtime: binding.runtime, daemon_id: "different-engine".into(), observed_unix_ms: 1,
                        }),
                    };
                    assert!(store.report_termination(source, agent.id, proof.clone()).is_err());
                    store.scheduler_tick().unwrap();
                    assert!(store.pending(target).unwrap().is_empty());
                    proof.proof.as_mut().unwrap().daemon_id = binding.daemon_id;
                    store.report_termination(source, agent.id, proof).unwrap();
                    store.observe_automatic_runner(source, true, true, true).unwrap();
                    store.scheduler_tick().unwrap();
                }
            }
            let pending = store.pending(target).unwrap();
            let [replacement]: [RunnerCommand; 1] = pending.try_into().unwrap();
            assert_eq!(replacement.ownership_epoch, 2);
            assert_eq!(replacement.stream_id, agent.stream_id);
            let spec = replacement.workspace_spec.as_ref().unwrap();
            assert_eq!(spec.setup.as_ref().unwrap().repository_source, CHECKPOINT_REPOSITORY);
            assert_eq!(spec.checkpoint.as_ref().unwrap().command_id, checkpoint.id);
            assert!(store.acknowledge(source, agent.id, ack(&agent)).is_err());
            assert!(store.observe_agent(source, agent.id, start()).is_err());
            store.acknowledge(target, replacement.id, ack(&replacement)).unwrap();
            assert_eq!(store.download_checkpoint_bundle(target, replacement.id, 2).unwrap(), bundle);
            complete(&replacement, CommandOutput::Workspace { workspace: Workspace {
                path: "/target/worktree".into(), branch: spec.branch.clone(), base_commit: "b".repeat(40),
            }});
            assert_eq!(store.snapshot().unwrap().tickets.iter()
                .find(|t| t.id == ticket.id).unwrap().execution_command, Some(replacement.id));
            store.scheduler_tick().unwrap();
            let [remote_checkpoint]: [RunnerCommand; 1] = store.pending(target).unwrap().try_into().unwrap();
            assert_eq!(remote_checkpoint.kind, CommandKind::Checkpoint);
            store.acknowledge(target, remote_checkpoint.id, ack(&remote_checkpoint)).unwrap();
            // An unavailable backup must not prevent otherwise valid local execution.
            complete(&remote_checkpoint, CommandOutput::CheckpointError {
                checkpoint_error: "Test bundle unavailable".into(),
            });
            store.scheduler_tick().unwrap();
            let [remote_agent]: [RunnerCommand; 1] = store.pending(target).unwrap().try_into().unwrap();
            assert_eq!(remote_agent.kind, CommandKind::RunAgent);
            assert_eq!(remote_agent.ownership_epoch, 2);
            if !accepted {
                let queue = || store.save_task(SaveTask {
                    request_id: RequestId::new(), id: None, expected_revision: Revision(0),
                    project_id: project.id, requested_session: None,
                    prompt: "A new intent while the laptop is offline".into(), images: vec![],
                }).unwrap();
                let offline_ticket = queue();
                store.scheduler_tick().unwrap();
                let [controller]: [RunnerCommand; 1] = store.pending(target).unwrap().into_iter()
                    .filter(|c| c.kind == CommandKind::PrepareWorkspace)
                    .collect::<Vec<_>>().try_into().unwrap();
                let controller_spec = controller.workspace_spec.as_ref().unwrap();
                assert_eq!(controller_spec.checkpoint.as_ref().unwrap().command_id, checkpoint.id);
                assert_eq!(controller_spec.setup.as_ref().unwrap().repository_source, CHECKPOINT_REPOSITORY);
                store.acknowledge(target, controller.id, ack(&controller)).unwrap();
                complete(&controller, CommandOutput::Workspace { workspace: Workspace {
                    path: "/target/orchestrator".into(), branch: controller_spec.branch.clone(), base_commit: "b".repeat(40),
                }});
                store.scheduler_tick().unwrap();
                let routing = store.pending(target).unwrap().into_iter()
                    .find(|c| c.stream_id == controller.stream_id).unwrap();
                store.acknowledge(target, routing.id, ack(&routing)).unwrap();
                complete(&routing, CommandOutput::Agent { agent: AgentProgress {
                    session_id: routing.session_id(), message_id: agent_message_id(routing.id),
                    phase: AgentPhase::Succeeded, summary: r#"{"action":"new","reason":"Independent work"}"#.into(),
                }});
                store.scheduler_tick().unwrap();
                let snapshot = store.snapshot().unwrap();
                let work = snapshot.tickets.iter().find(|t| t.id == offline_ticket.id).unwrap().execution_command.unwrap();
                let work = snapshot.commands.iter().find(|c| c.id == work).unwrap();
                assert_eq!(work.runner_id, target);
                assert_eq!(work.workspace_spec.as_ref().unwrap().checkpoint.as_ref().unwrap().command_id, checkpoint.id);

                store.acknowledge(target, remote_agent.id, ack(&remote_agent)).unwrap();
                complete(&remote_agent, CommandOutput::Agent { agent: AgentProgress {
                    session_id: remote_agent.session_id(), message_id: agent_message_id(remote_agent.id),
                    phase: AgentPhase::Succeeded, summary: "Made newer changes".into(),
                }});
                let too_old = queue();
                store.scheduler_tick().unwrap();
                let snapshot = store.snapshot().unwrap();
                let blocked = snapshot.tickets.iter().find(|t| t.id == too_old.id).unwrap();
                assert!(blocked.execution_command.is_none());
                assert!(blocked.routing_note.contains("ausreichend aktueller Checkpoint"));
                let export = store.pending(target).unwrap().into_iter()
                    .find(|c| c.stream_id == remote_agent.stream_id && c.kind == CommandKind::Checkpoint)
                    .expect("a successful imported result is exported without stopping the native runner");
                let export_spec = store.checkpoint(export.id).unwrap().unwrap();
                assert!(export_spec.committed_only);
                assert_eq!(export_spec.project_result.unwrap().runner_id, source);
                store.acknowledge(target, export.id, ack(&export)).unwrap();
                let receipt = store.upload_checkpoint_bundle(target, export.id, 2, b"result fixture".to_vec()).unwrap();
                complete(&export, CommandOutput::Checkpoint { checkpoint: GitCheckpoint {
                    commit: "d".repeat(40), parent: "b".repeat(40), tree: "e".repeat(40),
                    bundle_path: "/target/result.bundle".into(), manifest_path: "/target/result.json".into(),
                    bundle_sha256: receipt.sha256, bundle_bytes: receipt.bytes,
                }});
                store.scheduler_tick().unwrap();
                assert!(store.pending(target).unwrap().iter().any(|c|
                    c.kind == CommandKind::PrepareWorkspace && c.workspace_spec.as_ref()
                        .and_then(|spec| spec.checkpoint.as_ref())
                        .is_some_and(|seed| seed.command_id == export.id)));
                assert!(store.pending(source).unwrap().is_empty());
                store.observe_automatic_runner(source, true, true, true).unwrap();
                store.scheduler_tick().unwrap();
                let returned = store.pending(source).unwrap().into_iter().find(|command|
                    command.workspace_spec.as_ref().and_then(|spec| spec.setup.as_ref())
                        .is_some_and(|setup| setup.repository_source == CHECKPOINT_RETURN)).unwrap();
                let spec = returned.workspace_spec.as_ref().unwrap();
                assert_eq!(spec.setup.as_ref().unwrap().repository_path, "/desktop/private/checkout");
                assert_eq!(spec.checkpoint.as_ref().unwrap().command_id, export.id);
                store.acknowledge(source, returned.id, ack(&returned)).unwrap();
                assert_eq!(store.download_checkpoint_bundle(source, returned.id, 1).unwrap(), b"result fixture");
                complete(&returned, CommandOutput::Workspace { workspace: Workspace {
                    path: "/desktop/private/checkout".into(), branch: "main".into(), base_commit: "d".repeat(40),
                }});
                store.scheduler_tick().unwrap();
                assert!(!store.pending(source).unwrap().iter().any(|command|
                    command.workspace_spec.as_ref().and_then(|spec| spec.setup.as_ref())
                        .is_some_and(|setup| setup.repository_source == CHECKPOINT_RETURN)));
                let mut db = store.0.lock().unwrap();
                let tx = db.transaction().unwrap();
                let outdated = super::super::work::stream(&tx, work.stream_id).unwrap();
                assert!(!super::super::checkpoints::offline_seed_is_current(&tx, &outdated).unwrap());
                assert!(portable_source(&tx, project.id, 0).unwrap().is_none(),
                    "After a successful return, the reachable checkout is authoritative again");
            }
        }
    }

    #[test]
    fn managed_results_fan_out_to_each_reachable_mirror_and_offline_devices_catch_up() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let source = RunnerId::new();
        let first = RunnerId::new();
        let second = RunnerId::new();
        let setup = store.setup_project(SetupProject {
            key: None,
            request_id: RequestId::new(),
            project_id: None,
            expected_revision: Revision(0),
            name: "Managed mirrors".into(),
            repository: String::new(),
            base_branch: "main".into(),
            profile: ExecutionProfile::default(),
            setup: ProjectSetup {
                repository_source: String::new(),
                runner_id: source,
                repository_path: String::new(),
                service_registration: String::new(),
                harness: None,
            },
        }).unwrap();
        let ack = |command: &RunnerCommand| AckCommand {
            request_id: RequestId::new(),
            ownership_epoch: command.ownership_epoch,
        };
        store.acknowledge(source, setup.command.id, ack(&setup.command)).unwrap();
        store.complete(source, setup.command.id, CommandResult {
            request_id: RequestId::new(),
            ownership_epoch: setup.command.ownership_epoch,
            result: CommandOutput::Workspace { workspace: Workspace {
                path: "/source/worktree".into(),
                branch: setup.command.workspace_spec.as_ref().unwrap().branch.clone(),
                base_commit: "a".repeat(40),
            }},
        }).unwrap();
        let project = store.update_project(setup.project.id, UpdateProject {
            logo: None,
            runner_mapping: Some(ProjectRunnerMapping::Automatic),
            key: None,
            request_id: RequestId::new(),
            expected_revision: setup.project.revision,
            name: None,
            repository: None,
            base_branch: None,
        }).unwrap();
        store.set_scheduler(project.id, SetScheduler {
            request_id: RequestId::new(),
            expected_revision: Revision(0),
            enabled: true,
            runner_ids: vec![source],
            max_parallel: 1,
        }).unwrap();
        let project = store.snapshot().unwrap().projects.into_iter()
            .find(|candidate| candidate.id == project.id).unwrap();
        let project = store.set_project_mirror(project.id, SetProjectMirror {
            request_id: RequestId::new(),
            expected_revision: project.revision,
            runner_id: first,
            path: "/first/mirror".into(),
        }).unwrap();
        store.set_project_mirror(project.id, SetProjectMirror {
            request_id: RequestId::new(),
            expected_revision: project.revision,
            runner_id: second,
            path: "/second/mirror".into(),
        }).unwrap();

        let result = |parent: char, commit: char, bytes: &[u8]| {
            let checkpoint = {
                let mut db = store.0.lock().unwrap();
                let tx = db.transaction().unwrap();
                let owner = super::super::work::stream(&tx, setup.stream.id).unwrap();
                let checkpoint = Store::enqueue_portable_result(&tx, &owner, ProjectCheckpointOrigin {
                    runner_id: source,
                    repository: "factory:managed".into(),
                    repository_path: String::new(),
                    base_branch: "main".into(),
                }).unwrap();
                tx.commit().unwrap();
                checkpoint
            };
            store.acknowledge(source, checkpoint.id, ack(&checkpoint)).unwrap();
            let receipt = store.upload_checkpoint_bundle(
                source, checkpoint.id, checkpoint.ownership_epoch, bytes.to_vec(),
            ).unwrap();
            store.complete(source, checkpoint.id, CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: checkpoint.ownership_epoch,
                result: CommandOutput::Checkpoint { checkpoint: GitCheckpoint {
                    commit: commit.to_string().repeat(40),
                    parent: parent.to_string().repeat(40),
                    tree: "e".repeat(40),
                    bundle_path: "/source/result.bundle".into(),
                    manifest_path: "/source/result.json".into(),
                    bundle_sha256: receipt.sha256,
                    bundle_bytes: receipt.bytes,
                }},
            }).unwrap();
            checkpoint
        };

        let older = result('a', 'b', b"older result");
        store.observe_automatic_runner(first, true, true, true).unwrap();
        store.scheduler_tick().unwrap();
        let first_return = store.pending(first).unwrap().into_iter()
            .find(|command| command.workspace_spec.as_ref().and_then(|spec| spec.setup.as_ref())
                .is_some_and(|setup| setup.repository_source == CHECKPOINT_RETURN))
            .unwrap();
        assert_eq!(
            first_return.workspace_spec.as_ref().unwrap().checkpoint.as_ref().unwrap().command_id,
            older.id,
        );
        assert!(store.pending(second).unwrap().is_empty());

        let latest = result('b', 'c', b"latest result");
        store.scheduler_tick().unwrap();
        let first_latest = store.pending(first).unwrap().into_iter()
            .find(|command| command.workspace_spec.as_ref().and_then(|spec| spec.checkpoint.as_ref())
                .is_some_and(|checkpoint| checkpoint.command_id == latest.id))
            .unwrap();
        assert_ne!(first_latest.id, first_return.id);
        store.observe_automatic_runner(second, true, true, true).unwrap();
        store.scheduler_tick().unwrap();
        let second_latest = store.pending(second).unwrap().into_iter()
            .find(|command| command.workspace_spec.as_ref().and_then(|spec| spec.checkpoint.as_ref())
                .is_some_and(|checkpoint| checkpoint.command_id == latest.id))
            .unwrap();
        assert_eq!(
            second_latest.workspace_spec.as_ref().unwrap().setup.as_ref().unwrap().repository_path,
            "/second/mirror",
        );
        store.acknowledge(second, second_latest.id, ack(&second_latest)).unwrap();
        assert_eq!(
            store.download_checkpoint_bundle(second, second_latest.id, second_latest.ownership_epoch).unwrap(),
            b"latest result",
        );
    }

    #[test]
    fn auto_replaces_a_stale_local_runner_without_changing_manual_assignments() {
        for repository in ["/project", "C:/project", r"C:\project", r"\\host\share\project"] {
            assert_automatic_runner_replacement(repository);
        }
    }

    fn assert_automatic_runner_replacement(repository: &str) {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let project = store.create_project(CreateProject {
            request_id: RequestId::new(), key: None, logo: None,
            name: "Automatic runner".into(),
            repository: repository.into(),
            base_branch: "main".into(), runner_mapping: None,
        }).unwrap();
        let old = RunnerId::new();
        let remote = RunnerId::new();
        let local = RunnerId::new();
        store.save_task(SaveTask {
            request_id: RequestId::new(), id: None, expected_revision: Revision(0),
            project_id: project.id, requested_session: None,
            prompt: "Run on an available host".into(), images: vec![],
        }).unwrap();
        store.autostart_runner(old).unwrap();
        let snapshot = store.snapshot().unwrap();
        assert_eq!(snapshot.projects.iter().find(|p| p.id == project.id).unwrap().repository, repository);
        let assert_runner = |runner| {
            assert_eq!(store.scheduler_policy(project.id).unwrap().unwrap().runner_ids, [runner]);
        };
        store.observe_automatic_runner(remote, true, false, false).unwrap();
        store.scheduler_tick().unwrap();
        assert_runner(old);
        store.observe_automatic_runner(local, true, true, false).unwrap();
        store.scheduler_tick().unwrap();
        assert_runner(local);
        assert!(store.pending(local).unwrap().iter().any(|c| c.kind == CommandKind::PrepareWorkspace));
        let p = store.snapshot().unwrap().projects.into_iter().find(|p| p.id == project.id).unwrap();
        assert_eq!(p.runner_mapping, Some(ProjectRunnerMapping::Automatic));
        store.update_project(p.id, UpdateProject {
            request_id: RequestId::new(), expected_revision: p.revision,
            name: None, key: None, logo: None, repository: None, base_branch: None,
            runner_mapping: Some(ProjectRunnerMapping::Runner(old)),
        }).unwrap();
        store.scheduler_tick().unwrap();
        assert_runner(old);
    }
}

impl Store {
    #[cfg(test)]
    fn observe_automatic_runner(&self, id: RunnerId, ready: bool, local_source: bool, checkpoint: bool) -> Result<()> {
        self.observe_automatic_runner_location(id, ready, local_source, checkpoint, false)
    }

    pub(crate) fn observe_automatic_runner_location(
        &self, id: RunnerId, ready: bool, local_source: bool, checkpoint: bool, server_runner: bool,
    ) -> Result<()> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let now = super::super::unix_ms();
        db.execute("INSERT INTO automatic_runner_contacts VALUES(?1,?2)
            ON CONFLICT(runner_id) DO UPDATE SET seen_ms=excluded.seen_ms",
            params![id.to_string(), now])?;
        let previously_ready: Option<(bool, bool, bool)> = db.query_row(
            "SELECT local_source,checkpoint,server_runner FROM live_automatic_runners WHERE runner_id=?1 AND seen_ms >= ?2",
            params![id.to_string(), now.saturating_sub(30_000)], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        db.execute("DELETE FROM live_automatic_runners WHERE seen_ms < ?1", [now.saturating_sub(60_000)])?;
        if ready {
            db.execute("INSERT INTO live_automatic_runners VALUES(?1,?2,?3,?4,?5)
                ON CONFLICT(runner_id) DO UPDATE SET seen_ms=excluded.seen_ms,local_source=excluded.local_source,
                    checkpoint=excluded.checkpoint,server_runner=excluded.server_runner",
                params![id.to_string(), now, local_source, checkpoint, server_runner])?;
        } else {
            db.execute("DELETE FROM live_automatic_runners WHERE runner_id=?1", [id.to_string()])?;
        }
        if previously_ready != ready.then_some((local_source, checkpoint, server_runner)) {
            self.wake_scheduler();
        }
        Ok(())
    }

    pub fn recover_local_projects(&self, runner: RunnerId) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let commands: Vec<RunnerCommand> = {
            let mut stmt=tx.prepare("SELECT c.data FROM local_project_recovery r JOIN commands c ON c.id=r.command_id WHERE r.replacement_id IS NULL AND c.runner_id=?1")?;
            let rows = stmt.query_map([runner.to_string()], |r| r.get::<_, String>(0))?;
            rows.map(|r| Ok(serde_json::from_str(&r?)?))
                .collect::<Result<Vec<_>>>()?
        };
        for old in commands {
            let owner = super::work::stream(&tx, old.stream_id)?;
            let latest: String = tx.query_row(
                "SELECT id FROM commands WHERE stream_id=?1 ORDER BY rowid DESC LIMIT 1",
                [owner.id.to_string()],
                |r| r.get(0),
            )?;
            if latest != old.id.to_string()
                || owner.runner_id != runner
                || owner.ownership_epoch != old.ownership_epoch
            {
                continue;
            }
            tx.execute_batch("SAVEPOINT local_project_retry")?;
            match Self::enqueue_in(
                &tx,
                owner.id,
                &EnqueueCommand {
                    request_id: RequestId::new(),
                    expected_revision: owner.revision,
                    kind: CommandKind::PrepareWorkspace,
                    prompt: None,
                },
                None,
                false,
            ) {
                Ok(new) => {
                    // Never replay a previous attempt against changed project configuration.
                    if new.workspace_spec != old.workspace_spec {
                        tx.execute_batch("ROLLBACK TO local_project_retry")?;
                    } else {
                        tx.execute("UPDATE local_project_recovery SET replacement_id=?1 WHERE command_id=?2",params![new.id.to_string(),old.id.to_string()])?;
                        tx.execute("UPDATE scheduler_runs SET prepare_id=?1 WHERE prepare_id=?2 AND agent_id IS NULL",params![new.id.to_string(),old.id.to_string()])?;
                        event(
                            &tx,
                            "project.folder_retry",
                            owner.id.to_string(),
                            &serde_json::json!({"previous_command":old.id,"replacement_command":new.id}),
                        )?;
                    }
                }
                Err(
                    StoreError::WorkConflict
                    | StoreError::Invalid(_)
                    | StoreError::RevisionConflict(_),
                ) => {
                    tx.execute_batch("ROLLBACK TO local_project_retry")?;
                }
                Err(e) => return Err(e),
            }
            tx.execute_batch("RELEASE local_project_retry")?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn autostart_runner(&self, runner: RunnerId) -> Result<()> {
        self.autostart_runner_at(runner, false)
    }

    pub(crate) fn autostart_runner_at(&self, runner: RunnerId, server_runner: bool) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let projects: Vec<Project> = records(&tx, "SELECT p.data FROM active_projects p
            LEFT JOIN scheduler_policies s ON s.project_id=p.id
            WHERE s.project_id IS NULL OR (
                json_extract(p.data,'$.runner_mapping')='automatic'
                AND json_array_length(s.data,'$.runner_ids')=0
            ) ORDER BY p.rowid")?;
        for mut p in projects {
            let target = if let Some(ProjectRunnerMapping::Runner(target)) = p.runner_mapping {
                target
            } else if let (None, Some(setup)) = (p.runner_mapping, &p.setup) {
                setup.runner_id
            } else {
                // A server registration cannot claim an unbound desktop path.
                if server_runner && super::work::absolute_runner_path(&p.repository) {
                    continue;
                }
                let tasks: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM tickets WHERE project_id=?1)",
                    [p.id.to_string()],
                    |r| r.get(0),
                )?;
                let used: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM streams WHERE project_id=?1)",
                    [p.id.to_string()],
                    |r| r.get(0),
                )?;
                if p.setup.is_none() && (!tasks || used) {
                    continue;
                }
                if !p.repository.contains("://")
                    && !is_local_repository(&p.repository)
                    && !p.repository.contains('@')
                {
                    p.repository = "factory:managed".into();
                }
                bind_project_runner(&mut p, runner);
                p.revision.0 += 1;
                tx.execute(
                    "UPDATE projects SET data=?1 WHERE id=?2",
                    params![serde_json::to_string(&p)?, p.id.to_string()],
                )?;
                event(&tx, "project.automatic_setup", p.id.to_string(), &p)?;
                runner
            };
            // The selected host must actually be present before new work is assigned.
            if target != runner {
                continue;
            }
            if let Some(mut existing) = policy(&tx, p.id)? {
                existing.runner_ids = vec![runner];
                existing.revision.0 += 1;
                save_policy(&tx, &existing)?;
            } else {
                save_policy(&tx, &SchedulerPolicy {
                    project_id: p.id,
                    revision: Revision(1),
                    enabled: true,
                    runner_ids: vec![runner],
                    max_parallel: p.max_sessions,
                })?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn scheduler_policy(&self, id: ProjectId) -> Result<Option<SchedulerPolicy>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        policy(&tx, id)
    }
    pub fn set_scheduler(&self, id: ProjectId, b: SetScheduler) -> Result<SchedulerPolicy> {
        self.mutate(
            b.request_id,
            &format!("scheduler.configure/{id}"),
            &b,
            |tx| {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM active_projects WHERE id=?1)",
                    [id.to_string()],
                    |r| r.get(0),
                )?;
                if !exists {
                    return Err(StoreError::NotFound);
                }
                if b.enabled {
                    let data: String = tx.query_row(
                        "SELECT data FROM active_projects WHERE id=?1",
                        [id.to_string()],
                        |r| r.get(0),
                    )?;
                    let project: Project = serde_json::from_str(&data)?;
                    if let Some(model) = super::models::role_selection(tx, id, "implementer")? {
                        super::models::require_available(tx, &model)?;
                    } else if project.setup.is_none()
                        || !project.profile.provider_id.is_empty()
                        || !project.profile.model_id.is_empty()
                    {
                        line(&project.profile.provider_id, 120)?;
                        line(&project.profile.model_id, 200)?;
                    }
                }
                if b.runner_ids.is_empty()
                    || b.runner_ids.len() > 16
                    || b.runner_ids.iter().collect::<BTreeSet<_>>().len() != b.runner_ids.len()
                    || !(1..=16).contains(&b.max_parallel)
                {
                    return Err(StoreError::Invalid(
                        "Scheduler needs 1..16 unique runners and parallelism 1..16",
                    ));
                }
                let previous = policy(tx, id)?.map_or(Revision(0), |p| p.revision);
                if previous != b.expected_revision {
                    return Err(StoreError::RevisionConflict(previous));
                }
                let p = SchedulerPolicy {
                    project_id: id,
                    revision: Revision(previous.0 + 1),
                    enabled: b.enabled,
                    runner_ids: b.runner_ids.clone(),
                    max_parallel: b.max_parallel,
                };
                save_policy(tx, &p)?;
                let raw: String = tx.query_row(
                    "SELECT data FROM active_projects WHERE id=?1",
                    [id.to_string()],
                    |r| r.get(0),
                )?;
                let mut project: Project = serde_json::from_str(&raw)?;
                if project.max_sessions != p.max_parallel {
                    project.max_sessions = p.max_parallel;
                    project.revision.0 += 1;
                    tx.execute(
                        "UPDATE projects SET data=?1 WHERE id=?2",
                        params![serde_json::to_string(&project)?, id.to_string()],
                    )?;
                    event(tx, "project.session_limit", id.to_string(), &project)?;
                }
                Ok(p)
            },
        )
    }
    pub fn set_dependencies(&self, id: TicketId, b: SetDependencies) -> Result<Ticket> {
        self.mutate(
            b.request_id,
            &format!("ticket.dependencies/{id}"),
            &b,
            |tx| {
                let data: Option<String> = tx
                    .query_row(
                        "SELECT data FROM tickets WHERE id=?1",
                        [id.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?;
                let mut t: Ticket = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if t.revision != b.expected_revision {
                    return Err(StoreError::RevisionConflict(t.revision));
                }
                if b.depends_on.len() > 100
                    || b.depends_on.iter().collect::<BTreeSet<_>>().len() != b.depends_on.len()
                {
                    return Err(StoreError::Invalid("Invalid dependency count or duplicate"));
                }
                let assigned: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM stream_tickets WHERE ticket_id=?1)",
                    [id.to_string()],
                    |r| r.get(0),
                )?;
                if assigned {
                    return Err(StoreError::WorkConflict);
                }
                let all = tickets(tx, t.project_id)?;
                let mut seen = BTreeSet::new();
                let mut stack = b.depends_on.clone();
                while let Some(next) = stack.pop() {
                    if next == id {
                        return Err(StoreError::Invalid("Dependency cycle"));
                    }
                    if !seen.insert(next) {
                        continue;
                    }
                    let dep = all.get(&next).ok_or(StoreError::Invalid(
                        "Dependencies must belong to the same project",
                    ))?;
                    stack.extend(&dep.depends_on);
                }
                t.depends_on = b.depends_on.clone();
                save_ticket(tx, &mut t)?;
                Ok(t)
            },
        )
    }
    pub fn scheduler_tick(&self) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut stmt = tx.prepare("SELECT data FROM scheduler_policies ORDER BY rowid")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        for raw in rows {
            let mut p: SchedulerPolicy = serde_json::from_str(&raw)?;
            if !p.enabled {
                continue;
            }
            let (paused, mapped, automatic): (bool, bool, bool) = tx.query_row(
                "SELECT COALESCE(json_extract(data,'$.paused'),0), json_extract(data,'$.runner_mapping') IS NOT NULL,
                 COALESCE(json_extract(data,'$.runner_mapping')='automatic',0) FROM active_projects WHERE id=?1",
                [p.project_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            if paused {
                continue;
            }
            let mut all = tickets(&tx, p.project_id)?;
            if let Some(ticket) = all.values().filter(|t| !t.draft && t.requested_session.is_none()
                && matches!(t.status, TicketState::Backlog | TicketState::Ready)
                && t.depends_on.iter().all(|id| all.get(id).is_some_and(|t| t.status == TicketState::Done)))
                .min_by_key(|t| (std::cmp::Reverse(t.priority), t.created_unix_ms, t.id))
            {
                refresh_automatic_runner(&tx, &mut p, ticket)?;
            }
            tx.execute_batch("SAVEPOINT question_orchestration")?;
            match super::question_orchestrator::tick(&tx, &p) {
                Ok(()) => {}
                Err(
                    StoreError::WorkConflict
                    | StoreError::Invalid(_)
                    | StoreError::RevisionConflict(_),
                ) => tx.execute_batch("ROLLBACK TO question_orchestration")?,
                Err(error) => return Err(error),
            }
            tx.execute_batch("RELEASE question_orchestration")?;
            tx.execute_batch("SAVEPOINT workspace_recovery")?;
            match super::workspace_recovery::tick(&tx, &p) {
                Ok(()) => {}
                Err(StoreError::WorkConflict | StoreError::Invalid(_) | StoreError::RevisionConflict(_)) =>
                    tx.execute_batch("ROLLBACK TO workspace_recovery")?,
                Err(error) => return Err(error),
            }
            tx.execute_batch("RELEASE workspace_recovery")?;
            let since = super::super::unix_ms().saturating_sub(30_000);
            if automatic {
                tx.execute_batch("SAVEPOINT automatic_failover")?;
                match move_automatic_work(&tx, p.project_id, since) {
                    Ok(()) => {}
                    Err(StoreError::WorkConflict | StoreError::Invalid(_) | StoreError::RevisionConflict(_)) =>
                        tx.execute_batch("ROLLBACK TO automatic_failover")?,
                    Err(error) => return Err(error),
                }
                tx.execute_batch("RELEASE automatic_failover")?;
                export_portable_results(&tx, p.project_id, since)?;
                return_portable_result(&tx, p.project_id, since)?;
                return_project_mirrors(&tx, p.project_id, since)?;
            }
            // Recovery may update the execution command or commission a repair.
            all = tickets(&tx, p.project_id)?;
            // Advance only our own prepared streams; never adopt a manual assignment.
            let mut stmt=tx.prepare("SELECT r.ticket_id,r.stream_id,r.prepare_id,r.prompt FROM scheduler_runs r JOIN tickets t ON t.id=r.ticket_id WHERE t.project_id=?1 AND r.agent_id IS NULL ORDER BY r.rowid")?;
            let preparing = stmt
                .query_map([p.project_id.to_string()], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            drop(stmt);
            for (tid, sid, cid, prompt) in preparing {
                let tid: TicketId = tid.parse().map_err(|_| StoreError::Internal)?;
                let sid: StreamId = sid.parse().map_err(|_| StoreError::Internal)?;
                let Some(t) = all.get(&tid) else {
                    continue;
                };
                if t.status != TicketState::InProgress
                    || !t
                        .depends_on
                        .iter()
                        .all(|id| all.get(id).is_some_and(|t| t.status == TicketState::Done))
                {
                    continue;
                }
                let owner = super::work::stream(&tx, sid)?;
                // Remapping controls new assignments, not work already pinned to a runner.
                if !mapped && !p.runner_ids.contains(&owner.runner_id) {
                    continue;
                }
                if !super::checkpoints::offline_seed_is_current(&tx, &owner)? {
                    let mut ticket = t.clone();
                    super::routing::note(&tx, &mut ticket, "Auto wartet auf einen aktuellen Projekt-Checkpoint, bevor der vorbereitete Auftrag startet.")?;
                    continue;
                }
                let latest = latest_command(&tx, sid)?;
                let target_git = super::routing::assigned_git(&tx, tid)?;
                let baseline = if latest.kind == CommandKind::Checkpoint {
                    super::checkpoints::load(&tx, latest.id)?.is_some_and(|spec| spec.committed_only)
                } else {
                    false
                };
                if latest.id.to_string() != cid && !baseline
                    || !matches!(latest.state, CommandState::Succeeded | CommandState::Failed)
                    || (latest.state == CommandState::Failed && !baseline)
                {
                    continue;
                }
                let source_ready = checkpoint_runner_ready(&tx, owner.runner_id, since)?;
                let preferred_server = if automatic && baseline && source_ready && owner.ownership_epoch == 1
                    && target_git.is_none() && latest.state == CommandState::Succeeded
                {
                    let source_is_server: bool = tx.query_row("SELECT server_runner FROM live_automatic_runners
                        WHERE runner_id=?1", [owner.runner_id.to_string()], |row| row.get(0))?;
                    if source_is_server { None } else { automatic_target(&tx, owner.runner_id, t, since, true)? }
                } else {
                    None
                };
                if automatic && baseline && (!source_ready || preferred_server.is_some()) {
                    if latest.state == CommandState::Succeeded {
                        tx.execute_batch("SAVEPOINT checkpoint_handoff")?;
                        match reassign_checkpoint_to(&tx, owner, t, &latest, None, since, preferred_server) {
                            Ok(_) => {}
                            Err(StoreError::WorkConflict | StoreError::Invalid(_) | StoreError::RevisionConflict(_)) =>
                                tx.execute_batch("ROLLBACK TO checkpoint_handoff")?,
                            Err(error) => return Err(error),
                        }
                        tx.execute_batch("RELEASE checkpoint_handoff")?;
                    }
                    continue;
                }
                if automatic && !baseline && target_git.is_none() && source_ready
                    && latest.workspace_spec.as_ref().and_then(|spec| spec.setup.as_ref()).is_some()
                {
                    tx.execute_batch("SAVEPOINT automatic_checkpoint")?;
                    match Self::enqueue_in(&tx, sid, &EnqueueCommand {
                        request_id: RequestId::new(), expected_revision: owner.revision,
                        kind: CommandKind::Checkpoint, prompt: None,
                    }, None, false) {
                        Ok(checkpoint) => super::checkpoints::mark_committed_only(&tx, checkpoint.id)?,
                        Err(StoreError::WorkConflict | StoreError::Invalid(_) | StoreError::RevisionConflict(_)) =>
                            tx.execute_batch("ROLLBACK TO automatic_checkpoint")?,
                        Err(error) => return Err(error),
                    }
                    tx.execute_batch("RELEASE automatic_checkpoint")?;
                    continue;
                }
                // A rejected transition must not prevent other projects from advancing.
                tx.execute_batch("SAVEPOINT scheduled_start")?;
                let pins = super::models::ticket(&tx, tid)?;
                let repo_skills = super::skills::for_ticket(&tx, tid, t.project_id)?;
                let local_mcps = super::mcp::for_ticket(&tx, tid, t.project_id)?;
                match Self::enqueue_with_pins(
                    &tx,
                    sid,
                    &EnqueueCommand {
                        request_id: RequestId::new(),
                        expected_revision: owner.revision,
                        kind: CommandKind::RunAgent,
                        prompt: Some(prompt),
                    },
                    None,
                    "",
                    pins.as_ref().and_then(|m| m.implementer.as_ref()),
                    Some((&repo_skills, &local_mcps)),
                    target_git,
                ) {
                    Ok(c) => {
                        let mut ticket = t.clone();
                        ticket.execution_command = Some(c.id);
                        save_ticket(&tx, &mut ticket)?;
                        tx.execute(
                            "UPDATE scheduler_runs SET agent_id=?1 WHERE ticket_id=?2",
                            params![c.id.to_string(), tid.to_string()],
                        )?;
                        event(&tx, "scheduler.started", tid.to_string(), &c)?;
                    }
                    Err(StoreError::Invalid(reason))
                        if super::token_budgets::budget_reason(reason) =>
                    {
                        tx.execute_batch("ROLLBACK TO scheduled_start")?;
                        let mut ticket = t.clone();
                        super::routing::note(&tx,&mut ticket,"Tokenbudget blockiert: Limit erreicht oder Verbrauch unvollstaendig. Projektverbrauch (u) und Tokenlimits (b) pruefen.")?;
                    }
                    Err(
                        StoreError::WorkConflict
                        | StoreError::Invalid(_)
                        | StoreError::RevisionConflict(_),
                    ) => {
                        tx.execute_batch("ROLLBACK TO scheduled_start")?;
                    }
                    Err(e) => return Err(e),
                }
                tx.execute_batch("RELEASE scheduled_start")?;
            }
            if p.runner_ids.is_empty() {
                continue;
            }
            let mut stmt = tx.prepare("SELECT data FROM streams WHERE project_id=?1")?;
            let streams = stmt
                .query_map([p.project_id.to_string()], |r| r.get::<_, String>(0))?
                .map(|r| Ok(serde_json::from_str::<Workstream>(&r?)?))
                .collect::<Result<Vec<_>>>()?;
            drop(stmt);
            let mut counts: BTreeMap<RunnerId, usize> =
                p.runner_ids.iter().map(|r| (*r, 0)).collect();
            let mut active = 0;
            for s in &streams {
                let controller: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM orchestrators WHERE stream_id=?1 UNION ALL SELECT 1 FROM supervisor_policies WHERE stream_id=?1)",
                    [s.id.to_string()],
                    |r| r.get(0),
                )?;
                if controller {
                    continue;
                }
                let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands WHERE stream_id=?1 AND state IN ('pending','accepted'))",[s.id.to_string()],|r|r.get(0))?;
                let finished:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM scheduler_runs r JOIN commands c ON c.id=COALESCE(r.agent_id,r.prepare_id) WHERE r.stream_id=?1 AND (c.state IN ('failed','cancelled') OR (r.agent_id IS NOT NULL AND c.state='succeeded')))",[s.id.to_string()],|r|r.get(0))?;
                if busy
                    || s.ticket_ids
                        .iter()
                        .any(|id| all.get(id).is_some_and(|t| !done(t) && !finished))
                {
                    active += 1;
                    *counts.entry(s.runner_id).or_default() += 1;
                }
            }
            let mut ready = all
                .values()
                .filter(|t| {
                    !t.draft
                        && matches!(t.status, TicketState::Backlog | TicketState::Ready)
                        && t.depends_on
                            .iter()
                            .all(|id| all.get(id).is_some_and(|t| t.status == TicketState::Done))
                })
                .cloned()
                .collect::<Vec<_>>();
            ready.sort_by_key(|t| (std::cmp::Reverse(t.priority), t.created_unix_ms, t.id));
            for mut t in ready {
                if active >= p.max_parallel as usize {
                    break;
                }
                let assigned: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM stream_tickets WHERE ticket_id=?1)",
                    [t.id.to_string()],
                    |r| r.get(0),
                )?;
                if assigned {
                    continue;
                }
                let local_mcps = super::mcp::for_ticket(&tx, t.id, t.project_id)?;
                if !super::mcp::allowed(&tx, t.project_id, &local_mcps)? {
                    super::routing::note(&tx, &mut t, "MCP-Freigabe oder Konfiguration geändert; Aufgabe mit aktueller Auswahl neu einreihen")?;
                    continue;
                }
                let portable = if automatic { portable_source(&tx, p.project_id, since)? } else { None };
                let (runner, seed) = if let Some((project, source, offline)) = portable {
                    let Some(seed) = super::checkpoints::offline_project_seed(&tx, &project, &source)? else {
                        super::routing::note(&tx, &mut t, "Auto wartet auf die Projektquelle: Kein bestätigter, ausreichend aktueller Checkpoint verfügbar.")?;
                        continue;
                    };
                    let preferred = automatic_target(&tx, source.runner_id, &t, since, true)?;
                    let runner = if preferred.is_some() {
                        preferred
                    } else if !offline && checkpoint_runner_ready(&tx, source.runner_id, since)?
                        && super::mcp::runner_ready(&tx, source.runner_id, &local_mcps)?
                    {
                        Some(source.runner_id)
                    } else {
                        failover_target(&tx, source.runner_id, &t, since)?
                    };
                    let Some(runner) = runner else {
                        super::routing::note(&tx, &mut t, "Auto wartet auf einen verfügbaren Runner für den bestätigten Projektstand.")?;
                        continue;
                    };
                    (runner, Some(seed))
                } else {
                    let mut eligible = Vec::new();
                    for candidate in &p.runner_ids {
                        if super::mcp::runner_ready(&tx, *candidate, &local_mcps)? {
                            eligible.push(*candidate);
                        }
                    }
                    let Some(runner) = eligible.into_iter()
                        .min_by_key(|r| (counts.get(r).copied().unwrap_or(0), *r))
                    else {
                        super::routing::note(&tx, &mut t, "Wartet auf verfügbaren Runner mit den freigegebenen MCPs")?;
                        continue;
                    };
                    (runner, None)
                };
                let mut prompt = t.prompt.clone().unwrap_or_else(|| {
                    format!(
                        "Work on ticket {}: {}\nDescription:\n{}\nAcceptance criteria:\n{}",
                        t.id, t.title, t.description, t.acceptance_criteria
                    )
                });
                if prompt.trim().is_empty() && (!t.images.is_empty() || !t.files.is_empty()) {
                    prompt =
                        "Bearbeite die Aufgabe anhand der angehängten Bilder und Dateien.".into();
                }
                let goal = t.title.chars().take(60).collect::<String>();
                let mut routing_policy = p.clone();
                if seed.is_some() {
                    routing_policy.runner_ids = vec![runner];
                }
                tx.execute_batch("SAVEPOINT routing_evaluation")?;
                let routed = match super::routing::route(&tx, &routing_policy, &mut t, seed.as_ref()) {
                    Ok(route) => route,
                    Err(StoreError::Invalid(reason))
                        if super::token_budgets::budget_reason(reason) =>
                    {
                        tx.execute_batch("ROLLBACK TO routing_evaluation")?;
                        t = all.get(&t.id).ok_or(StoreError::Internal)?.clone();
                        super::routing::note(&tx,&mut t,"Orchestrierungs-Tokenbudget blockiert: Projektverbrauch (u) und Tokenlimits (b) pruefen.")?;
                        super::routing::Route::Pending
                    }
                    Err(
                        StoreError::WorkConflict
                        | StoreError::Invalid(_)
                        | StoreError::RevisionConflict(_),
                    ) => {
                        tx.execute_batch("ROLLBACK TO routing_evaluation")?;
                        t = all.get(&t.id).ok_or(StoreError::Internal)?.clone();
                        super::routing::note(&tx,&mut t,"Orchestrator kann noch nicht starten; Projektprofil, Runner-Zuordnung und Steuerung pruefen")?;
                        super::routing::Route::Pending
                    }
                    Err(e) => return Err(e),
                };
                tx.execute_batch("RELEASE routing_evaluation")?;
                let mut direct_git = None;
                match routed {
                    super::routing::Route::Pending => continue,
                    super::routing::Route::Session(root) => {
                        if seed.is_some() {
                            super::routing::note(&tx, &mut t, "Auto wartet auf die ursprüngliche Session; native Sessions werden nicht auf andere Runner umgeleitet.")?;
                            continue;
                        }
                        if let Some(runner) =
                            super::routing::continue_task(&tx, &mut t, root, &prompt)?
                        {
                            all.insert(t.id, t.clone());
                            active += 1;
                            *counts.entry(runner).or_default() += 1;
                        }
                        continue;
                    }
                    super::routing::Route::New => {}
                    super::routing::Route::TargetGit { worktree_prompt, git } => {
                        let raw: String = tx.query_row("SELECT data FROM active_projects WHERE id=?1",
                            [t.project_id.to_string()], |r| r.get(0))?;
                        let project: Project = serde_json::from_str(&raw)?;
                        if seed.is_some()
                            || !project.setup.as_ref().is_some_and(|s| std::path::Path::new(&s.repository_source).is_absolute())
                            || !project.runner_bindings.get(&runner).is_some_and(|b| b.repository_path.is_empty())
                        {
                            super::routing::note(&tx, &mut t, "Orchestrator wartet: Git-Schritte benötigen einen lokalen Ziel-Checkout auf dem Runner")?;
                            continue;
                        }
                        if let Some(work) = worktree_prompt {
                            prompt = work;
                        } else {
                            direct_git = Some(git);
                        }
                        super::routing::note(&tx, &mut t, "Orchestrator: Git-Schritte laufen im Ziel-Checkout")?;
                    }
                }
                let mut stream = Self::create_stream_in(
                    &tx,
                    &CreateStream {
                        request_id: RequestId::new(),
                        project_id: p.project_id,
                        runner_id: runner,
                        goal,
                        ticket_ids: vec![t.id],
                    },
                )?;
                super::checkpoints::seed_stream(&tx, &mut stream, seed.as_ref())?;
                let direct = direct_git.is_some();
                let c = Self::enqueue_with_pins(
                    &tx,
                    stream.id,
                    &EnqueueCommand {
                        request_id: RequestId::new(),
                        expected_revision: stream.revision,
                        kind: if direct { CommandKind::TargetGit } else { CommandKind::PrepareWorkspace },
                        prompt: None,
                    },
                    None,
                    "",
                    None,
                    None,
                    direct_git,
                )?;
                tx.execute("INSERT INTO scheduler_runs(ticket_id,stream_id,prepare_id,prompt,agent_id) VALUES(?1,?2,?3,?4,?5)",params![t.id.to_string(),stream.id.to_string(),c.id.to_string(),prompt,direct.then(|| c.id.to_string())])?;
                t.status = TicketState::InProgress;
                t.execution_command = Some(c.id);
                if seed.is_some() {
                    t.routing_note = "Auto: Neuer Auftrag verwendet den letzten bestätigten Projektstand aus der Factory.".into();
                }
                save_ticket(&tx, &mut t)?;
                all.insert(t.id, t.clone());
                active += 1;
                *counts.entry(runner).or_default() += 1;
                event(
                    &tx,
                    "scheduler.assigned",
                    t.id.to_string(),
                    &serde_json::json!({"ticket":t.id,"stream":stream.id,"runner":runner,"policy_revision":p.revision}),
                )?;
            }
            tx.execute_batch("SAVEPOINT memory_extraction")?;
            match super::memory::tick(&tx, &p) {
                Ok(()) => {}
                Err(
                    StoreError::WorkConflict
                    | StoreError::Invalid(_)
                    | StoreError::RevisionConflict(_),
                ) => tx.execute_batch("ROLLBACK TO memory_extraction")?,
                Err(error) => return Err(error),
            }
            tx.execute_batch("RELEASE memory_extraction")?;
        }
        tx.commit()?;
        Ok(())
    }
}
