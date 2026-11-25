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
pub(super) fn set_limit(tx: &Transaction<'_>, id: ProjectId, limit: u8) -> Result<()> {
    if let Some(mut p) = policy(tx, id)? {
        p.max_parallel = limit;
        p.revision.0 += 1;
        save_policy(tx, &p)?;
    }
    Ok(())
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
    let local_repository = std::path::Path::new(&project.repository).is_absolute()
        || project.repository.as_bytes().get(1..3).is_some_and(|prefix| prefix == b":\\" || prefix == b":/")
        || project.repository.starts_with("\\\\");
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

#[cfg(test)]
mod automatic_runner_tests {
    use super::*;

    #[test]
    fn auto_replaces_a_stale_local_runner_without_changing_manual_assignments() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let project = store.create_project(CreateProject {
            request_id: RequestId::new(), key: None, logo: None,
            name: "Automatic runner".into(), repository: "/project".into(),
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
        let assert_runner = |runner| {
            assert_eq!(store.scheduler_policy(project.id).unwrap().unwrap().runner_ids, [runner]);
        };
        store.observe_automatic_runner(remote, true, false).unwrap();
        store.scheduler_tick().unwrap();
        assert_runner(old);
        store.observe_automatic_runner(local, true, true).unwrap();
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
    pub(crate) fn observe_automatic_runner(&self, id: RunnerId, ready: bool, local_source: bool) -> Result<()> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let now = super::super::unix_ms();
        let previously_ready: Option<bool> = db.query_row(
            "SELECT local_source FROM live_automatic_runners WHERE runner_id=?1 AND seen_ms >= ?2",
            params![id.to_string(), now.saturating_sub(30_000)], |row| row.get(0),
        ).optional()?;
        db.execute("DELETE FROM live_automatic_runners WHERE seen_ms < ?1", [now.saturating_sub(60_000)])?;
        if ready {
            db.execute("INSERT INTO live_automatic_runners VALUES(?1,?2,?3)
                ON CONFLICT(runner_id) DO UPDATE SET seen_ms=excluded.seen_ms,local_source=excluded.local_source",
                params![id.to_string(), now, local_source])?;
        } else {
            db.execute("DELETE FROM live_automatic_runners WHERE runner_id=?1", [id.to_string()])?;
        }
        if previously_ready != ready.then_some(local_source) {
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
                    && !p.repository.starts_with('/')
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
            let (paused, mapped): (bool, bool) = tx.query_row(
                "SELECT COALESCE(json_extract(data,'$.paused'),0), json_extract(data,'$.runner_mapping') IS NOT NULL FROM active_projects WHERE id=?1",
                [p.project_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
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
                let latest: String = tx.query_row(
                    "SELECT id FROM commands WHERE stream_id=?1 ORDER BY rowid DESC LIMIT 1",
                    [sid.to_string()],
                    |r| r.get(0),
                )?;
                if latest != cid {
                    continue;
                }
                let state: String =
                    tx.query_row("SELECT state FROM commands WHERE id=?1", [cid], |r| {
                        r.get(0)
                    })?;
                if state != "succeeded" {
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
                    super::routing::assigned_git(&tx, tid)?,
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
                let mut eligible = Vec::new();
                for candidate in &p.runner_ids {
                    if super::mcp::runner_ready(&tx, *candidate, &local_mcps)? {
                        eligible.push(candidate);
                    }
                }
                let Some(&runner) = eligible.into_iter()
                    .min_by_key(|r| (counts.get(r).copied().unwrap_or(0), **r))
                else {
                    super::routing::note(&tx, &mut t, "Wartet auf verfügbaren Runner mit den freigegebenen MCPs")?;
                    continue;
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
                tx.execute_batch("SAVEPOINT routing_evaluation")?;
                let routed = match super::routing::route(&tx, &p, &mut t) {
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
                        if !project.setup.as_ref().is_some_and(|s| std::path::Path::new(&s.repository_source).is_absolute())
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
                let stream = Self::create_stream_in(
                    &tx,
                    &CreateStream {
                        request_id: RequestId::new(),
                        project_id: p.project_id,
                        runner_id: runner,
                        goal,
                        ticket_ids: vec![t.id],
                    },
                )?;
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
