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
pub struct Store(Arc<Mutex<Connection>>);

pub struct DurableSnapshot {
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
            version <= 21,
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
        tx.commit()?;
        Ok(Self(Arc::new(Mutex::new(db))))
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
            let project = Project {
                runner_bindings: Default::default(),
                setup: None,
                id: ProjectId::new(),
                revision: Revision(1),
                name: body.name.clone(),
                repository: body.repository.clone(),
                base_branch: body.base_branch.clone(),
                created_unix_ms: super::unix_ms(),
                profile: ExecutionProfile::default(),
            };
            tx.execute(
                "INSERT INTO projects(id,data) VALUES(?1,?2)",
                params![project.id.to_string(), serde_json::to_string(&project)?],
            )?;
            event(tx, "project.created", project.id.to_string(), &project)?;
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
                "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1)",
                [body.project_id.to_string()],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(StoreError::NotFound);
            }
            let now = super::unix_ms();
            let ticket = Ticket {
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
                        "SELECT data FROM projects WHERE id=?1",
                        [id.to_string()],
                        |r| r.get(0),
                    )
                    .optional()?;
                let mut project: Project =
                    serde_json::from_str(&data.ok_or(StoreError::NotFound)?)?;
                if project.revision != body.expected_revision {
                    return Err(StoreError::RevisionConflict(project.revision));
                }
                if body.name.is_none() && body.repository.is_none() && body.base_branch.is_none() {
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
                if let Some(v) = &body.repository {
                    project.repository = v.clone();
                }
                if let Some(v) = &body.base_branch {
                    project.base_branch = v.clone();
                }
                project_fields(&project.name, &project.repository, &project.base_branch)?;
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
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction()?;
        let cursor = tx.query_row("SELECT COALESCE(MAX(sequence),0) FROM events", [], |r| {
            r.get(0)
        })?;
        let projects = records(&tx, "SELECT data FROM projects ORDER BY rowid")?;
        let tickets = records(&tx, "SELECT data FROM tickets ORDER BY rowid")?;
        let streams = records(&tx, "SELECT data FROM streams ORDER BY rowid")?;
        let commands = records(&tx, "SELECT data FROM commands ORDER BY rowid")?;
        let questions = records(&tx, "SELECT data FROM questions ORDER BY rowid")?;
        let controls = records(&tx, "SELECT data FROM controls ORDER BY rowid")?;
        let scheduler_policies =
            records(&tx, "SELECT data FROM scheduler_policies ORDER BY rowid")?;
        tx.commit()?;
        Ok(DurableSnapshot {
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

mod archive;
mod checkpoints;
mod compaction;
mod controls;
mod devices;
mod excerpts;
mod knowledge;
mod questions;
mod reviews;
mod scheduler;
mod sessions;
mod setup;
mod supervisor;
mod termination;
mod work;

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
