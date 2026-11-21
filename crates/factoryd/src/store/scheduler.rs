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
impl Store {
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
                    "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
                    [id.to_string()],
                    |r| r.get(0),
                )?;
                if !exists {
                    return Err(StoreError::NotFound);
                }
                if b.enabled {
                    let data: String = tx.query_row(
                        "SELECT data FROM projects WHERE id=?1",
                        [id.to_string()],
                        |r| r.get(0),
                    )?;
                    let project: Project = serde_json::from_str(&data)?;
                    line(&project.profile.provider_id, 120)?;
                    line(&project.profile.model_id, 200)?;
                }
                if b.runner_ids.is_empty()
                    || b.runner_ids.len() > 16
                    || b.runner_ids.iter().collect::<BTreeSet<_>>().len() != b.runner_ids.len()
                    || !(1..=16).contains(&b.max_parallel)
                    || b.start_budget > 1000
                {
                    return Err(StoreError::Invalid(
                        "Scheduler needs 1..16 unique runners, parallelism 1..16, budget <=1000",
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
                    remaining_starts: b.start_budget,
                };
                save_policy(tx, &p)?;
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
            let mut all = tickets(&tx, p.project_id)?;
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
                if !p.runner_ids.contains(&owner.runner_id) {
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
                match Self::enqueue_in(
                    &tx,
                    sid,
                    &EnqueueCommand {
                        request_id: RequestId::new(),
                        expected_revision: owner.revision,
                        kind: CommandKind::RunAgent,
                        prompt: Some(prompt),
                    },
                    None,
                    false,
                ) {
                    Ok(c) => {
                        tx.execute(
                            "UPDATE scheduler_runs SET agent_id=?1 WHERE ticket_id=?2",
                            params![c.id.to_string(), tid.to_string()],
                        )?;
                        event(&tx, "scheduler.started", tid.to_string(), &c)?;
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
                let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands WHERE stream_id=?1 AND state IN ('pending','accepted'))",[s.id.to_string()],|r|r.get(0))?;
                if busy
                    || s.ticket_ids
                        .iter()
                        .any(|id| all.get(id).is_some_and(|t| !done(t)))
                {
                    active += 1;
                    *counts.entry(s.runner_id).or_default() += 1;
                }
            }
            let mut ready = all
                .values()
                .filter(|t| {
                    t.status == TicketState::Ready
                        && t.depends_on
                            .iter()
                            .all(|id| all.get(id).is_some_and(|t| t.status == TicketState::Done))
                })
                .cloned()
                .collect::<Vec<_>>();
            ready.sort_by_key(|t| (std::cmp::Reverse(t.priority), t.created_unix_ms, t.id));
            for mut t in ready {
                if active >= p.max_parallel as usize || p.remaining_starts == 0 {
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
                let runner = *p
                    .runner_ids
                    .iter()
                    .min_by_key(|r| (counts.get(r).copied().unwrap_or(0), **r))
                    .unwrap();
                let prompt = format!(
                    "Work on ticket {}: {}\nDescription:\n{}\nAcceptance criteria:\n{}",
                    t.id, t.title, t.description, t.acceptance_criteria
                );
                if prompt.len() > 16384 {
                    t.status = TicketState::Blocked;
                    save_ticket(&tx, &mut t)?;
                    event(
                        &tx,
                        "scheduler.blocked",
                        t.id.to_string(),
                        &serde_json::json!({"reason":"Ticket prompt exceeds 16384 UTF-8 bytes"}),
                    )?;
                    continue;
                }
                let goal = t.title.chars().take(60).collect::<String>();
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
                let c = Self::enqueue_in(
                    &tx,
                    stream.id,
                    &EnqueueCommand {
                        request_id: RequestId::new(),
                        expected_revision: stream.revision,
                        kind: CommandKind::PrepareWorkspace,
                        prompt: None,
                    },
                    None,
                    false,
                )?;
                tx.execute("INSERT INTO scheduler_runs(ticket_id,stream_id,prepare_id,prompt) VALUES(?1,?2,?3,?4)",params![t.id.to_string(),stream.id.to_string(),c.id.to_string(),prompt])?;
                t.status = TicketState::InProgress;
                save_ticket(&tx, &mut t)?;
                all.insert(t.id, t.clone());
                p.remaining_starts -= 1;
                p.revision.0 += 1;
                active += 1;
                *counts.entry(runner).or_default() += 1;
                event(
                    &tx,
                    "scheduler.assigned",
                    t.id.to_string(),
                    &serde_json::json!({"ticket":t.id,"stream":stream.id,"runner":runner,"policy_revision":p.revision}),
                )?;
                save_policy(&tx, &p)?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
