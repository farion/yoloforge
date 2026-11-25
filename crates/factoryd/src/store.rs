//! SQLite writes, request deduplication and event creation share one IMMEDIATE transaction.
use factory_core::*;
use factory_protocol::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Debug)]
pub enum StoreError {
    Invalid(&'static str),
    NotFound,
    RequestConflict,
    RevisionConflict(Revision),
    Internal,
    WorkConflict,
}
type Result<T> = std::result::Result<T, StoreError>;
pub const SCHEMA_VERSION: u32 = 42;
impl From<rusqlite::Error> for StoreError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Internal
    }
}
impl From<serde_json::Error> for StoreError {
    fn from(_: serde_json::Error) -> Self {
        Self::Internal
    }
}

#[derive(Clone)]
pub struct Store(Arc<Mutex<Connection>>, Arc<tokio::sync::Notify>);

pub struct DurableSnapshot {
    pub token_totals: ObservedTokenTotals,
    pub session_activity_unix_ms: std::collections::BTreeMap<String, u64>,
    pub editable_tasks: std::collections::BTreeMap<TicketId, Revision>,
    pub session_titles: std::collections::BTreeMap<String, String>,
    pub session_closures: Vec<SessionClosure>,
    pub compactions: Vec<Compaction>,
    pub runtime_terminations: Vec<RuntimeTermination>,
    pub token_budgets: Vec<TokenBudget>,
    pub scheduler_policies: Vec<SchedulerPolicy>,
    pub projects: Vec<Project>,
    pub tickets: Vec<Ticket>,
    pub cursor: u64,
    pub streams: Vec<Workstream>,
    pub commands: Vec<RunnerCommand>,
    pub questions: Vec<Question>,
    pub controls: Vec<StreamControl>,
}

impl Store {
    pub fn scheduler_wakeup(&self) -> Arc<tokio::sync::Notify> {
        self.1.clone()
    }

    fn wake_scheduler(&self) {
        self.1.notify_one();
    }

    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(3))?;
        db.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
        )?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        anyhow::ensure!(
            version <= SCHEMA_VERSION,
            "database schema is newer than this factoryd supports"
        );
        if version == 0 {
            tx.execute_batch(include_str!("../migrations/001_initial.sql"))?;
        }
        if version < 2 {
            tx.execute_batch(include_str!("../migrations/002_work.sql"))?;
        }
        if version < 3 {
            tx.execute_batch(include_str!("../migrations/003_questions.sql"))?;
        }
        if version < 4 {
            tx.execute_batch(include_str!("../migrations/004_controls.sql"))?;
        }
        if version < 5 {
            tx.execute_batch(include_str!("../migrations/005_excerpts.sql"))?;
        }
        if version < 6 {
            tx.execute_batch(include_str!("../migrations/006_devices.sql"))?;
        }
        if version < 7 {
            tx.execute_batch(include_str!("../migrations/007_knowledge.sql"))?;
        }
        if version < 8 {
            tx.execute_batch(include_str!("../migrations/008_archive.sql"))?;
        }
        if version < 9 {
            tx.execute_batch(include_str!("../migrations/009_compaction.sql"))?;
        }
        if version < 10 {
            tx.execute_batch(include_str!("../migrations/010_session_closures.sql"))?;
        }
        if version < 11 {
            tx.execute_batch(include_str!("../migrations/011_supervisor.sql"))?;
        }
        if version < 12 {
            tx.execute_batch(include_str!("../migrations/012_knowledge_scope.sql"))?;
        }
        if version < 13 {
            tx.execute_batch(include_str!("../migrations/013_containment.sql"))?;
        }
        if version < 14 {
            tx.execute_batch(include_str!("../migrations/014_control_recovery.sql"))?;
        }
        if version < 15 {
            tx.execute_batch(include_str!("../migrations/015_runtime_termination.sql"))?;
        }
        if version < 16 {
            tx.execute_batch(include_str!("../migrations/016_runtime_bindings.sql"))?;
        }
        if version < 17 {
            tx.execute_batch(include_str!("../migrations/017_checkpoints.sql"))?;
        }
        if version < 18 {
            tx.execute_batch(include_str!("../migrations/018_checkpoint_handoff.sql"))?;
        }
        if version < 19 {
            tx.execute_batch(include_str!("../migrations/019_reviews.sql"))?;
        }
        if version < 20 {
            tx.execute_batch(include_str!("../migrations/020_scheduler.sql"))?;
        }
        if version < 21 {
            tx.execute_batch("PRAGMA user_version=21;")?;
        }
        if version < 22 {
            tx.execute_batch("PRAGMA user_version=22;")?;
        }
        if version < 23 {
            tx.execute_batch("PRAGMA user_version=23;")?;
        }
        if version < 24 {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS local_project_recovery(command_id TEXT PRIMARY KEY,replacement_id TEXT);
                INSERT OR IGNORE INTO local_project_recovery(command_id) SELECT id FROM commands WHERE state='failed' AND json_extract(data,'$.kind')='prepare_workspace' AND json_extract(data,'$.workspace_spec.setup.repository_source') LIKE '/%' AND json_extract(data,'$.result.workspace_error') LIKE 'Repository import failed;%';
                PRAGMA user_version=24;")?;
        }
        if version < 25 {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS orchestrators(project_id TEXT PRIMARY KEY,stream_id TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS task_routing(ticket_id TEXT PRIMARY KEY,data TEXT NOT NULL CHECK(json_valid(data)));
                UPDATE projects SET data=json_set(data,'$.max_sessions',COALESCE((SELECT json_extract(s.data,'$.max_parallel') FROM scheduler_policies s WHERE s.project_id=projects.id),1)) WHERE json_extract(data,'$.max_sessions') IS NULL;
                UPDATE tickets SET data=json_set(data,'$.execution_command',(SELECT COALESCE(r.agent_id,r.prepare_id) FROM scheduler_runs r WHERE r.ticket_id=tickets.id)) WHERE EXISTS(SELECT 1 FROM scheduler_runs r WHERE r.ticket_id=tickets.id);
                PRAGMA user_version=25;")?;
        }
        if version < 26 {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS question_evaluations(question_id TEXT PRIMARY KEY REFERENCES questions(id), data TEXT NOT NULL CHECK(json_valid(data))); PRAGMA user_version=26;")?;
        }
        if version < 27 {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS memory_evaluations(source_command_id TEXT PRIMARY KEY REFERENCES commands(id),data TEXT NOT NULL CHECK(json_valid(data))); PRAGMA user_version=27;")?;
        }
        if version < 28 {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS hosted_reviews(command_id TEXT PRIMARY KEY REFERENCES commands(id),data TEXT NOT NULL CHECK(json_valid(data))); PRAGMA user_version=28;")?;
        }
        if version < 29 {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS push_delivery(id INTEGER PRIMARY KEY CHECK(id=1), destination TEXT NOT NULL, cursor INTEGER NOT NULL DEFAULT 0, pending INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0, next_attempt INTEGER NOT NULL DEFAULT 0); CREATE INDEX IF NOT EXISTS events_question_push ON events(kind,sequence); PRAGMA user_version=29;")?;
        }
        if version < 30 {
            tx.execute_batch(include_str!("../migrations/030_token_budgets.sql"))?;
        }
        if version < 31 {
            tx.execute_batch(include_str!("../migrations/031_archive_blobs.sql"))?;
        }
        if version < 32 {
            tx.execute_batch(include_str!("../migrations/032_desktop.sql"))?;
        }
        if version < 33 {
            tx.execute_batch(include_str!("../migrations/033_task_edits.sql"))?;
        }
        if version < 34 {
            tx.execute_batch(include_str!("../migrations/034_ticket_numbers.sql"))?;
            numbering::migrate(&tx).map_err(|error| anyhow::anyhow!("Ticket numbering migration failed: {error:?}"))?;
        }
        if version < 35 {
            tx.execute_batch(include_str!("../migrations/035_models.sql"))?;
        }
        if version < 36 {
            tx.execute_batch(include_str!("../migrations/036_repo_skills.sql"))?;
        }
        if version < 37 {
            tx.execute_batch(include_str!("../migrations/037_local_mcps.sql"))?;
        }
        if version < 38 {
            tx.execute_batch(include_str!("../migrations/038_remote_mcps.sql"))?;
        }
        if version < 39 {
            tx.execute_batch(include_str!("../migrations/039_factory_skills.sql"))?;
        }
        if version < 40 {
            tx.execute_batch(include_str!("../migrations/040_remove_start_budget.sql"))?;
        }
        if version < 41 {
            tx.execute_batch(include_str!("../migrations/041_workspace_recovery.sql"))?;
        }
        if version < 42 {
            tx.execute_batch(include_str!("../migrations/042_runner_restart.sql"))?;
        }
        tx.commit()?;
        // Connection-local view centralizes fencing of removed projects without
        // changing or discarding the historical rows or the database schema version.
        db.execute_batch("CREATE TEMP VIEW active_projects AS SELECT rowid AS rowid,* FROM main.projects WHERE COALESCE(json_extract(data,'$.deleted'),0)=0;")?;
        db.execute_batch("CREATE TEMP TABLE live_mcp_runners(runner_id TEXT PRIMARY KEY, seen_ms INTEGER NOT NULL, remote INTEGER NOT NULL);")?;
        db.execute_batch("CREATE TEMP TABLE live_automatic_runners(runner_id TEXT PRIMARY KEY, seen_ms INTEGER NOT NULL, local_source INTEGER NOT NULL, checkpoint INTEGER NOT NULL, server_runner INTEGER NOT NULL);")?;
        db.execute_batch("CREATE TEMP TABLE automatic_runner_contacts(runner_id TEXT PRIMARY KEY, seen_ms INTEGER NOT NULL);")?;
        db.execute_batch("CREATE TEMP TABLE native_runner_boots(runner_id TEXT PRIMARY KEY, data TEXT NOT NULL);")?;
        Ok(Self(Arc::new(Mutex::new(db)), Arc::new(tokio::sync::Notify::new())))
    }

    fn mutate<T: Serialize + DeserializeOwned>(
        &self,
        id: RequestId,
        operation: &str,
        body: &impl Serialize,
        change: impl FnOnce(&Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut hash = Sha256::new();
        hash.update(operation.as_bytes());
        hash.update([0]);
        hash.update(serde_json::to_vec(body)?);
        let fingerprint = format!("{:x}", hash.finalize());
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<(String, String)> = tx
            .query_row(
                "SELECT fingerprint, response FROM requests WHERE id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((original, response)) = previous {
            if original != fingerprint {
                return Err(StoreError::RequestConflict);
            }
            return Ok(serde_json::from_str(&response)?);
        }
        let response = change(&tx)?;
        tx.execute(
            "INSERT INTO requests(id,fingerprint,response) VALUES(?1,?2,?3)",
            params![
                id.to_string(),
                fingerprint,
                serde_json::to_string(&response)?
            ],
        )?;
        tx.commit()?;
        Ok(response)
    }

    pub fn create_project(&self, body: CreateProject) -> Result<Project> {
        self.mutate(body.request_id, "project.create", &body, |tx| {
            project_fields(&body.name, &body.repository, &body.base_branch)?;
            if let Some(logo) = &body.logo {
                logo.png_bytes().map_err(StoreError::Invalid)?;
            }
            let mut project = Project {
                logo: body.logo.clone(),
                runner_mapping: None,
                key: numbering::project_key(tx,&body.name,body.key.as_deref(),None)?,
                deleted: false,
                max_sessions: default_session_limit(),
                paused: false,
                runner_bindings: Default::default(),
                mirrors: Default::default(),
                setup: None,
                id: ProjectId::new(),
                revision: Revision(1),
                name: body.name.clone(),
                repository: body.repository.clone(),
                base_branch: body.base_branch.clone(),
                created_unix_ms: super::unix_ms(),
                profile: ExecutionProfile::default(),
            };
            if let Some(mapping) = body.runner_mapping {
                scheduler::map_project_runner(tx, &mut project, mapping)?;
            }
            tx.execute(
                "INSERT INTO projects(id,data) VALUES(?1,?2)",
                params![project.id.to_string(), serde_json::to_string(&project)?],
            )?;
            event(tx, "project.created", project.id.to_string(), &project)?;
            numbering::reserve_key(tx,project.id,&project.key)?;
            Ok(project)
        })
    }

    pub fn create_ticket(&self, body: CreateTicket) -> Result<Ticket> {
        self.mutate(body.request_id, "ticket.create", &body, |tx| {
            ticket_fields(
                &body.title,
                &body.description,
                &body.acceptance_criteria,
                body.priority,
            )?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM active_projects WHERE id=?1)",
                [body.project_id.to_string()],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(StoreError::NotFound);
            }
            let now = super::unix_ms();
            let ticket = Ticket {
                key: numbering::next_reference(tx,body.project_id)?,
                draft: false,
                files: vec![],
                execution_command: None,
                requested_session: None,
                routing_note: String::new(),
                prompt: None,
                images: vec![],
                depends_on: vec![],
                id: TicketId::new(),
                project_id: body.project_id,
                revision: Revision(1),
                title: body.title.clone(),
                description: body.description.clone(),
                acceptance_criteria: body.acceptance_criteria.clone(),
                priority: body.priority,
                status: TicketState::Backlog,
                created_unix_ms: now,
                updated_unix_ms: now,
            };
            tx.execute(
                "INSERT INTO tickets(id,project_id,revision,data) VALUES(?1,?2,?3,?4)",
                params![
                    ticket.id.to_string(),
                    ticket.project_id.to_string(),
                    1,
                    serde_json::to_string(&ticket)?
                ],
            )?;
            event(tx, "ticket.created", ticket.id.to_string(), &ticket)?;
            Ok(ticket)
        })
    }

    pub fn update_project(&self, id: ProjectId, body: UpdateProject) -> Result<Project> {
        self.mutate(
            body.request_id,
            &format!("project.update/{id}"),
            &body,
            |tx| {
                let data: Option<String> = tx
                    .query_row(
                        "SELECT data FROM active_projects WHERE id=?1",
                        [id.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?;
                let mut project: Project =
                    serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if project.revision != body.expected_revision {
                    return Err(StoreError::RevisionConflict(project.revision));
                }
                if body.name.is_none() && body.repository.is_none() && body.base_branch.is_none() && body.key.is_none() && body.runner_mapping.is_none() && body.logo.is_none() {
                    return Err(StoreError::Invalid("Project update is empty"));
                }
                if body
                    .repository
                    .as_ref()
                    .is_some_and(|v| v != &project.repository)
                    || body
                        .base_branch
                        .as_ref()
                        .is_some_and(|v| v != &project.base_branch)
                {
                    let has_streams: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM streams WHERE project_id=?1)",
                        [id.to_string()],
                        |r| r.get(0),
                    )?;
                    if has_streams {
                        return Err(StoreError::WorkConflict);
                    }
                }
                if let Some(v) = &body.name {
                    project.name = v.clone();
                }
                if let Some(change) = &body.logo {
                    if let Some(logo) = &change.image {
                        logo.png_bytes().map_err(StoreError::Invalid)?;
                    }
                    project.logo = change.image.clone();
                }
                if let Some(key) = &body.key {
                    if key.trim().is_empty() {return Err(StoreError::Invalid("Projektkürzel darf nicht leer sein"));}
                    let key=numbering::project_key(tx,&project.name,Some(key),Some(id))?;
                    if key!=project.key {
                        numbering::rename(tx,id,&key)?;
                        project.key=key;
                    }
                }
                if let Some(v) = &body.repository {
                    project.repository = v.clone();
                    if let Some(setup) = &mut project.setup {
                        setup.repository_source = v.clone();
                    }
                }
                if let Some(v) = &body.base_branch {
                    project.base_branch = v.clone();
                }
                if project.repository != "factory:managed" && !project.mirrors.is_empty() {
                    return Err(StoreError::WorkConflict);
                }
                project_fields(&project.name, &project.repository, &project.base_branch)?;
                if let Some(mapping) = body.runner_mapping {
                    scheduler::map_project_runner(tx, &mut project, mapping)?;
                }
                project.revision.0 = project
                    .revision
                    .0
                    .checked_add(1)
                    .ok_or(StoreError::Internal)?;
                tx.execute(
                    "UPDATE projects SET data=?1 WHERE id=?2",
                    params![serde_json::to_string(&project)?, id.to_string()],
                )?;
                event(tx, "project.updated", id.to_string(), &project)?;
                Ok(project)
            },
        )
    }
    pub fn set_project_mirror(&self, id: ProjectId, body: SetProjectMirror) -> Result<Project> {
        self.mutate(body.request_id, &format!("project.mirror/{id}"), &body, |tx| {
            let data: Option<String> = tx.query_row(
                "SELECT data FROM active_projects WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            ).optional()?;
            let mut project: Project = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
            if project.revision != body.expected_revision {
                return Err(StoreError::RevisionConflict(project.revision));
            }
            update_mirror(&mut project, &body)?;
            project.revision.0 += 1;
            tx.execute(
                "UPDATE projects SET data=?1 WHERE id=?2",
                params![serde_json::to_string(&project)?, id.to_string()],
            )?;
            event(tx, "project.mirror", id.to_string(), &project)?;
            Ok(project)
        })
    }

    pub fn update_ticket(&self, id: TicketId, body: UpdateTicket) -> Result<Ticket> {
        self.mutate(
            body.request_id,
            &format!("ticket.update/{id}"),
            &body,
            |tx| {
                let data: Option<String> = tx
                    .query_row(
                        "SELECT data FROM tickets WHERE id=?1",
                        [id.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?;
                let mut ticket: Ticket = serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if ticket.revision != body.expected_revision {
                    return Err(StoreError::RevisionConflict(ticket.revision));
                }
                ticket.revision.0 = ticket
                    .revision
                    .0
                    .checked_add(1)
                    .ok_or(StoreError::Internal)?;
                if body.title.is_none()
                    && body.description.is_none()
                    && body.acceptance_criteria.is_none()
                    && body.priority.is_none()
                    && body.status.is_none()
                {
                    return Err(StoreError::Invalid("Ticket update is empty"));
                }
                if let Some(value) = &body.title {
                    ticket.title = value.clone();
                }
                if let Some(value) = &body.description {
                    ticket.description = value.clone();
                }
                if let Some(value) = &body.acceptance_criteria {
                    ticket.acceptance_criteria = value.clone();
                }
                if let Some(value) = body.priority {
                    ticket.priority = value;
                }
                if let Some(value) = body.status {
                    ticket.status = value;
                }
                ticket_fields(
                    &ticket.title,
                    &ticket.description,
                    &ticket.acceptance_criteria,
                    ticket.priority,
                )?;
                ticket.updated_unix_ms = super::unix_ms();
                let changed = tx.execute(
                    "UPDATE tickets SET revision=?1,data=?2 WHERE id=?3 AND revision=?4",
                    params![
                        ticket.revision.0,
                        serde_json::to_string(&ticket)?,
                        id.to_string(),
                        body.expected_revision.0
                    ],
                )?;
                if changed != 1 {
                    return Err(StoreError::Internal);
                }
                event(tx, "ticket.updated", id.to_string(), &ticket)?;
                Ok(ticket)
            },
        )
    }

    pub fn snapshot(&self) -> Result<DurableSnapshot> {
        self.snapshot_with_usage(false)
    }

    pub fn desktop_snapshot(&self) -> Result<DurableSnapshot> {
        self.snapshot_with_usage(true)
    }

    fn snapshot_with_usage(&self, include_usage: bool) -> Result<DurableSnapshot> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let cursor = tx.query_row("SELECT COALESCE(MAX(sequence),0) FROM events", [], |r| {
            r.get(0)
        })?;
        let projects = records(&tx, "SELECT data FROM active_projects ORDER BY rowid")?;
        let tickets: Vec<Ticket> = records(&tx, "SELECT data FROM tickets ORDER BY rowid")?;
        let mut editable_tasks = std::collections::BTreeMap::new();
        for ticket in &tickets {
            if tasks::editable(&tx, ticket)? {
                editable_tasks.insert(ticket.id, ticket.revision);
            }
        }
        let streams = records(&tx, "SELECT data FROM streams ORDER BY rowid")?;
        let commands: Vec<RunnerCommand> =
            records(&tx, "SELECT data FROM commands ORDER BY rowid")?;
        let questions = records(&tx, "SELECT data FROM questions ORDER BY rowid")?;
        let controls = records(&tx, "SELECT data FROM controls ORDER BY rowid")?;
        let scheduler_policies =
            records(&tx, "SELECT data FROM scheduler_policies ORDER BY rowid")?;
        let token_budgets = records(&tx, "SELECT data FROM token_budgets ORDER BY rowid")?;
        let session_closures = records(&tx, "SELECT data FROM session_closures ORDER BY rowid")?;
        let compactions = records(&tx, "SELECT data FROM compactions ORDER BY rowid")?;
        let runtime_terminations =
            records(&tx, "SELECT data FROM runtime_terminations ORDER BY rowid")?;
        let session_titles = {
            let mut stmt=tx.prepare("SELECT json_extract(e.data,'$.session_id'),json_extract(e.data,'$.session_title') FROM excerpts e JOIN commands c ON c.id=e.command_id WHERE json_type(e.data,'$.session_title')='text' ORDER BY json_extract(c.data,'$.created_unix_ms'),json_extract(e.data,'$.captured_unix_ms')")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut titles = std::collections::BTreeMap::new();
            for row in rows {
                let (id, title) = row?;
                if !title.trim().is_empty() {
                    titles.insert(id, title);
                }
            }
            titles
        };
        let session_activity_unix_ms = {
            let mut statement=tx.prepare(
                "SELECT c.id,MAX(e.created_unix_ms) FROM commands c JOIN events e ON e.aggregate_id=c.id AND e.kind IN ('command.created','command.updated') GROUP BY c.id
                 UNION ALL
                 SELECT q.command_id,MAX(e.created_unix_ms) FROM questions q JOIN events e ON e.aggregate_id=q.id AND e.kind IN ('question.created','question.updated') GROUP BY q.command_id")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })?;
            let mut by_command = std::collections::BTreeMap::<String, u64>::new();
            for row in rows {
                let (id, time) = row?;
                by_command
                    .entry(id)
                    .and_modify(|value| *value = (*value).max(time))
                    .or_insert(time);
            }
            let mut activity = std::collections::BTreeMap::<String, u64>::new();
            for command in commands.iter().filter(|c| c.kind == CommandKind::RunAgent) {
                let time = by_command
                    .get(&command.id.to_string())
                    .copied()
                    .unwrap_or(command.created_unix_ms)
                    .max(command.created_unix_ms);
                activity
                    .entry(command.session_id())
                    .and_modify(|value| *value = (*value).max(time))
                    .or_insert(time);
            }
            activity
        };
        let token_totals = if include_usage {
            usage::observed_token_totals(&tx, &commands)?
        } else {
            ObservedTokenTotals::default()
        };
        tx.commit()?;
        Ok(DurableSnapshot {
            token_totals,
            session_activity_unix_ms,
            editable_tasks,
            session_titles,
            session_closures,
            compactions,
            runtime_terminations,
            token_budgets,
            scheduler_policies,
            projects,
            tickets,
            cursor,
            streams,
            commands,
            questions,
            controls,
        })
    }

    pub fn events(&self, after: u64, limit: u32) -> Result<EventPage> {
        if after > i64::MAX as u64 || !(1..=500).contains(&limit) {
            return Err(StoreError::Invalid("Invalid event cursor or limit"));
        }
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let mut stmt = db.prepare("SELECT sequence,kind,aggregate_id,created_unix_ms,payload FROM events WHERE sequence>?1 ORDER BY sequence LIMIT ?2")?;
        let rows = stmt.query_map(params![after, limit], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (sequence, kind, aggregate_id, created_unix_ms, payload) = row?;
            events.push(FactoryEvent {
                sequence,
                kind,
                aggregate_id,
                created_unix_ms,
                payload: serde_json::from_str(&payload)?,
            });
        }
        let next_cursor = events.last().map_or(after, |e| e.sequence);
        Ok(EventPage {
            events,
            next_cursor,
        })
    }
}
mod models;
mod mcp;
mod remote_mcp;
mod skills;
mod factory_skills;

mod archive;
mod archive_blobs;
mod checkpoints;
mod compaction;
mod controls;
mod desktop;
mod devices;
mod excerpts;
mod knowledge;
mod memory;
mod push;
mod question_orchestrator;
mod questions;
mod reviews;
mod routing;
mod runner_restart;
mod scheduler;
mod sessions;
mod setup;
mod supervisor;
mod tasks;
mod task_history;
mod numbering;
mod termination;
mod token_budgets;
mod usage;
mod work;
mod workspace_recovery;

fn records<T: DeserializeOwned>(tx: &Transaction<'_>, sql: &str) -> Result<Vec<T>> {
    let mut stmt = tx.prepare(sql)?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(serde_json::from_str(&row?)?);
    }
    Ok(result)
}
fn event(tx: &Transaction<'_>, kind: &str, id: String, payload: &impl Serialize) -> Result<()> {
    tx.execute(
        "INSERT INTO events(kind,aggregate_id,created_unix_ms,payload) VALUES(?1,?2,?3,?4)",
        params![kind, id, super::unix_ms(), serde_json::to_string(payload)?],
    )?;
    if matches!(
        kind,
        "question.created" | "question.updated" | "command.updated" | "scheduler.blocked"
    ) {
        desktop::record_notification(tx, kind, &id, &serde_json::to_value(payload)?)?;
    }
    Ok(())
}
fn project_fields(name: &str, repository: &str, base_branch: &str) -> Result<()> {
    line(name, 120)?;
    line(repository, 2048)?;
    line(base_branch, 200)?;
    // Repository metadata is not a credential store. SSH usernames are allowed.
    if repository.contains("://") {
        let url = url::Url::parse(repository)
            .map_err(|_| StoreError::Invalid("Invalid repository URL"))?;
        if url.password().is_some()
            || (matches!(url.scheme(), "http" | "https") && !url.username().is_empty())
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(StoreError::Invalid(
                "Repository URL must not contain credentials, query or fragment",
            ));
        }
    }
    Ok(())
}
fn update_mirror(project: &mut Project, mirror: &SetProjectMirror) -> Result<()> {
    if project.repository != "factory:managed" {
        return Err(StoreError::Invalid("Local mirrors require a Factory-managed project"));
    }
    if mirror.path.is_empty() {
        project.mirrors.remove(&mirror.runner_id);
        return Ok(());
    }
    if mirror.path.len() > 4096
        || mirror.path.chars().any(char::is_control)
        || !work::absolute_runner_path(&mirror.path)
    {
        return Err(StoreError::Invalid("Mirror path must be absolute on its runner"));
    }
    project.mirrors.insert(mirror.runner_id, ProjectMirror { path: mirror.path.clone() });
    Ok(())
}

fn line(value: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(StoreError::Invalid(
            "Empty, oversized or invalid single-line field",
        ));
    }
    Ok(())
}
fn ticket_fields(title: &str, description: &str, criteria: &str, priority: u8) -> Result<()> {
    line(title, 240)?;
    if priority > 3
        || [description, criteria].iter().any(|s| {
            s.len() > 32768
                || s.chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        })
    {
        return Err(StoreError::Invalid("Invalid priority or multiline text"));
    }
    Ok(())
}
