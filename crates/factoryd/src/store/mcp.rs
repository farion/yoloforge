use super::*;
use factory_protocol::mcp::*;
use serde::Deserialize;
use std::collections::BTreeSet;

pub(super) fn catalog(db: &Connection) -> Result<LocalMcpCatalog> {
    let raw: Option<String> = db
        .query_row("SELECT data FROM local_mcp_catalog WHERE id=1", [], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(raw
        .map(|s| {
            let mut value: serde_json::Value = serde_json::from_str(&s)?;
            // Upgrade the mutable catalog, not already-pinned execution profiles.
            if let Some(entries) = value["entries"].as_array_mut() {
                for entry in entries {
                    if entry.get("all_runners").is_none() {
                        entry["all_runners"] = serde_json::json!(true);
                    }
                }
            }
            serde_json::from_value(value)
        })
        .transpose()?
        .unwrap_or(LocalMcpCatalog {
            revision: Revision(0),
            entries: Vec::new(),
        }))
}

fn project(db: &Connection, id: ProjectId) -> Result<ProjectLocalMcps> {
    let raw: Option<String> = db
        .query_row(
            "SELECT data FROM project_local_mcps WHERE project_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or(ProjectLocalMcps {
            revision: Revision(0),
            required: Vec::new(),
            optional: Vec::new(),
        }))
}

fn clean(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

impl Store {
    pub fn local_mcp_catalog(&self) -> Result<LocalMcpCatalog> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        catalog(&db)
    }

    pub fn save_local_mcp_catalog(&self, input: SetLocalMcpCatalog) -> Result<LocalMcpCatalog> {
        self.mutate(input.request_id, "mcp.local.catalog", &input, |tx| {
            let current = catalog(tx)?;
            if input.catalog.revision != current.revision {
                return Err(StoreError::RevisionConflict(current.revision));
            }
            if input.catalog.entries.len() > 32 {
                return Err(StoreError::Invalid("Maximal 32 lokale MCP-Server"));
            }
            let mut saved = input.catalog.clone();
            let mut ids = BTreeSet::new();
            for entry in &mut saved.entries {
                if !valid_repo_skill_id(&entry.id) || !ids.insert(entry.id.clone())
                    || !clean(&entry.name, 80) || entry.command.is_empty() || entry.command.len() > 32
                    || !Path::new(&entry.command[0]).is_absolute()
                    || entry.command.iter().any(|arg| arg.len() > 512 || arg.contains('{') || arg.chars().any(char::is_control))
                    || entry.runners.len() > 32 || entry.excluded_runners.len() > 256
                {
                    return Err(StoreError::Invalid("Ungültiger MCP: eindeutige ID, Name und absoluter Programmpfad erforderlich"));
                }
                entry.runners.sort_unstable();
                entry.runners.dedup();
                entry.excluded_runners.sort_unstable();
                entry.excluded_runners.dedup();
            }
            let remote = super::remote_mcp::entries(tx)?;
            if remote.iter().any(|m| ids.contains(&m.reference.id)) {
                return Err(StoreError::Invalid("Diese MCP-ID ist bereits als Remote-MCP vergeben"));
            }
            ids.extend(remote.into_iter().map(|m| m.reference.id));
            let mut statement = tx.prepare("SELECT m.data FROM project_local_mcps m JOIN active_projects p ON p.id=m.project_id")?;
            let used = statement.query_map([], |r| r.get::<_, String>(0))?;
            for row in used {
                let settings: ProjectLocalMcps = serde_json::from_str(&row?)?;
                if settings.required.iter().chain(&settings.optional).any(|id| !ids.contains(id)) {
                    return Err(StoreError::Invalid("MCP zuerst in allen Projekten entfernen"));
                }
            }
            saved.entries.sort_by(|a, b| a.id.cmp(&b.id));
            saved.revision.0 += 1;
            tx.execute("INSERT INTO local_mcp_catalog(id,data) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
                       [serde_json::to_string(&saved)?])?;
            // Do not send commands or arguments through the general event feed.
            event(tx, "mcp.catalog", "local".into(), &saved.revision)?;
            Ok(saved)
        })
    }

    pub fn project_local_mcps(&self, id: ProjectId) -> Result<ProjectLocalMcps> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        super::skills::check_project(&db, id)?;
        project(&db, id)
    }

    pub fn save_project_local_mcps(
        &self,
        id: ProjectId,
        input: SetProjectLocalMcps,
    ) -> Result<ProjectLocalMcps> {
        self.mutate(input.request_id, &format!("project.mcp.local/{id}"), &input, |tx| {
            super::skills::check_project(tx, id)?;
            let current = project(tx, id)?;
            if input.settings.revision != current.revision {
                return Err(StoreError::RevisionConflict(current.revision));
            }
            let mut saved = input.settings.clone();
            let available = catalog(tx)?;
            let remote = super::remote_mcp::entries(tx)?;
            saved.required.sort_unstable();
            saved.optional.sort_unstable();
            let wanted = saved.required.iter().chain(&saved.optional).collect::<Vec<_>>();
            if wanted.len() > 8 || wanted.iter().copied().collect::<BTreeSet<_>>().len() != wanted.len()
                || wanted.iter().any(|id| !available.entries.iter().any(|e| &e.id == *id)
                   && !remote.iter().any(|e| &e.reference.id == *id))
            {
                return Err(StoreError::Invalid("Maximal acht unterschiedliche MCPs aus dem Factory-Katalog wählen"));
            }
            saved.revision.0 += 1;
            tx.execute("INSERT INTO project_local_mcps(project_id,data) VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET data=excluded.data",
                       params![id.to_string(), serde_json::to_string(&saved)?])?;
            event(tx, "project.mcp.local", id.to_string(), &saved)?;
            Ok(saved)
        })
    }

    pub(crate) fn observe_mcp_runner(
        &self,
        id: RunnerId,
        available: bool,
        remote: bool,
    ) -> Result<()> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let now = super::super::unix_ms();
        db.execute(
            "DELETE FROM live_mcp_runners WHERE seen_ms < ?1",
            [now.saturating_sub(60_000)],
        )?;
        if available {
            db.execute("INSERT INTO live_mcp_runners VALUES(?1,?2,?3) ON CONFLICT(runner_id) DO UPDATE SET seen_ms=excluded.seen_ms,remote=excluded.remote",
                        params![id.to_string(), now, remote])?;
        } else {
            db.execute(
                "DELETE FROM live_mcp_runners WHERE runner_id=?1",
                [id.to_string()],
            )?;
        }
        Ok(())
    }

    pub(crate) fn require_local_mcp_support(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        supported: bool,
    ) -> Result<()> {
        if !supported {
            let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
            let tx = db.transaction()?;
            let command = super::work::command(&tx, id, runner, epoch)?;
            if command
                .agent_spec
                .as_ref()
                .is_some_and(|s| !s.local_mcps.is_empty())
            {
                return Err(StoreError::WorkConflict);
            }
        }
        Ok(())
    }

    pub(crate) fn require_remote_mcp_support(
        &self,
        runner: RunnerId,
        id: CommandId,
        epoch: u64,
        supported: bool,
    ) -> Result<()> {
        if !supported {
            let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
            let tx = db.transaction()?;
            if super::work::command(&tx, id, runner, epoch)?
                .agent_spec
                .as_ref()
                .is_some_and(|s| !s.remote_mcps.is_empty())
            {
                return Err(StoreError::WorkConflict);
            }
        }
        Ok(())
    }
}

pub(super) fn selected(db: &Connection, id: ProjectId) -> Result<McpProfile> {
    resolve(db, id, &[])
}

pub(super) fn resolve(db: &Connection, id: ProjectId, optional: &[String]) -> Result<McpProfile> {
    let grants = project(db, id)?;
    let mut wanted = grants.required;
    let allowed = optional.iter().collect::<BTreeSet<_>>();
    if allowed.len() != optional.len() || allowed.iter().any(|id| !grants.optional.contains(id)) {
        return Err(StoreError::Invalid(
            "Nicht freigegebener MCP in der Aufgabenauswahl",
        ));
    }
    wanted.extend(optional.iter().cloned());
    wanted.sort_unstable();
    if wanted.is_empty() {
        return Ok(McpProfile::default());
    }
    let available = catalog(db)?;
    let remote = super::remote_mcp::entries(db)?;
    let mut result = McpProfile::default();
    for id in wanted {
        if let Some(m) = available.entries.iter().find(|e| e.id == id) {
            result.local.push(m.clone());
        } else if let Some(m) = remote.iter().find(|e| e.reference.id == id) {
            result.remote.push(m.reference.clone());
        } else {
            return Err(StoreError::Internal);
        }
    }
    Ok(result)
}

#[derive(Serialize, Deserialize)]
struct TaskMcps {
    entries: Vec<LocalMcp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    remote: Vec<RemoteMcpRef>,
    optional: Vec<String>,
}

pub(super) fn pin(
    tx: &Transaction<'_>,
    ticket: TicketId,
    profile: McpProfile,
    optional: Vec<String>,
) -> Result<()> {
    tx.execute("INSERT INTO task_local_mcps(ticket_id,data) VALUES(?1,?2) ON CONFLICT(ticket_id) DO UPDATE SET data=excluded.data",
               params![ticket.to_string(), serde_json::to_string(&TaskMcps {entries:profile.local, remote:profile.remote, optional})?])?;
    Ok(())
}

fn task(db: &Connection, ticket: TicketId) -> Result<Option<TaskMcps>> {
    let raw: Option<String> = db
        .query_row(
            "SELECT data FROM task_local_mcps WHERE ticket_id=?1",
            [ticket.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
}

pub(super) fn for_ticket(
    db: &Connection,
    ticket: TicketId,
    project_id: ProjectId,
) -> Result<McpProfile> {
    match task(db, ticket)? {
        Some(s) => Ok(McpProfile {
            local: s.entries,
            remote: s.remote,
        }),
        None => selected(db, project_id),
    }
}

pub(super) fn task_optional(db: &Connection, ticket: TicketId) -> Result<Vec<String>> {
    Ok(task(db, ticket)?.map_or_else(Vec::new, |s| s.optional))
}

pub(super) fn default_optional(db: &Connection, project_id: ProjectId) -> Result<Vec<String>> {
    Ok(project(db, project_id)?.optional)
}

    pub(super) fn granted(db: &Connection, project_id: ProjectId, id: &str) -> Result<bool> {
    let grants = project(db, project_id)?;
    Ok(grants
        .required
        .iter()
        .chain(&grants.optional)
        .any(|entry| entry == id))
}

pub(super) fn allowed(db: &Connection, id: ProjectId, pinned: &McpProfile) -> Result<bool> {
    if pinned.is_empty() {
        return Ok(true);
    }
    let grants = project(db, id)?;
    let active = catalog(db)?;
    let remote = super::remote_mcp::entries(db)?;
    Ok(pinned.local.iter().all(|e| {
        (grants.required.contains(&e.id) || grants.optional.contains(&e.id))
            && active.entries.contains(e)
    }) && pinned.remote.iter().all(|e| {
        (grants.required.contains(&e.id) || grants.optional.contains(&e.id))
            && remote.iter().any(|m| &m.reference == e)
    }))
}

pub(super) fn runner_ready(db: &Connection, runner: RunnerId, pinned: &McpProfile) -> Result<bool> {
    if pinned.is_empty() {
        return Ok(true);
    }
    if !pinned.local.iter().all(|mcp| mcp.enabled_on(runner)) {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM live_mcp_runners WHERE runner_id=?1 AND seen_ms >= ?2 AND (NOT ?3 OR remote))",
        params![
            runner.to_string(),
            super::super::unix_ms().saturating_sub(30_000),
            !pinned.remote.is_empty()
        ],
        |r| r.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_protocol::models::SubmitTaskWithModel;

    fn fixture_program() -> String {
        std::env::current_exe().unwrap().into_os_string().into_string().unwrap()
    }

    #[test]
    fn catalog_defaults_to_all_runners_and_persists_explicit_exclusions() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let first = RunnerId::new();
        let future = RunnerId::new();
        let old = serde_json::json!({
            "revision": 0,
            "entries": [{"id":"test","name":"Test","command":[fixture_program()],"runners":[first]}]
        });
        store.0.lock().unwrap().execute(
            "INSERT INTO local_mcp_catalog(id,data) VALUES(1,?1)", [old.to_string()]
        ).unwrap();
        let mut catalog = store.local_mcp_catalog().unwrap();
        assert!(catalog.entries[0].enabled_on(first));
        assert!(catalog.entries[0].enabled_on(future));
        catalog.entries[0].excluded_runners = vec![first, first];
        store.save_local_mcp_catalog(SetLocalMcpCatalog {
            request_id: RequestId::new(), catalog,
        }).unwrap();
        let restored = store.local_mcp_catalog().unwrap();
        assert_eq!(restored.entries[0].excluded_runners, vec![first]);
        assert!(!restored.entries[0].enabled_on(first));
        assert!(restored.entries[0].enabled_on(future));
        assert!(restored.entries[0].enabled_on(RunnerId::new()));
        // Old immutable execution profiles keep their original allowlist.
        let pinned: LocalMcp = serde_json::from_value(old["entries"][0].clone()).unwrap();
        assert!(pinned.enabled_on(first));
        assert!(!pinned.enabled_on(future));
    }

    #[test]
    fn automatic_runner_policy_is_used_by_scheduling_and_respects_explicit_exclusions() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let runner = RunnerId::new();
        store.observe_mcp_runner(runner, true, false).unwrap();
        let mut pinned = McpProfile {
            local: vec![LocalMcp {
                id: "test".into(), name: "Test".into(), command: vec![fixture_program()],
                runners: vec![], all_runners: true, excluded_runners: vec![],
            }],
            ..Default::default()
        };
        let db = store.0.lock().unwrap();
        assert!(runner_ready(&db, runner, &pinned).unwrap());
        pinned.local[0].excluded_runners.push(runner);
        assert!(!runner_ready(&db, runner, &pinned).unwrap());
    }

    #[test]
    fn optional_mcp_selection_stays_draft_scoped_and_waits_for_its_runner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let store = Store::open(&path).unwrap();
        let project = store
            .create_project(CreateProject {
                logo: None,
                request_id: RequestId::new(),
                name: "Assets".into(),
                repository: "factory:managed".into(),
                base_branch: "main".into(),
                runner_mapping: None,
                key: None,
            })
            .unwrap();
        store
            .update_profile(
                project.id,
                UpdateProfile {
                    request_id: RequestId::new(),
                    expected_revision: project.revision,
                    profile: ExecutionProfile {
                        provider_id: "fixture".into(),
                        model_id: "test".into(),
                        ..project.profile
                    },
                },
            )
            .unwrap();
        let runner = RunnerId::new();
        let another = RunnerId::new();
        let blender = LocalMcp {
            all_runners: false, excluded_runners: vec![],
            id: "blender".into(),
            name: "Blender".into(),
            command: vec![fixture_program()],
            runners: vec![runner],
        };
        let catalog = store
            .save_local_mcp_catalog(SetLocalMcpCatalog {
                request_id: RequestId::new(),
                catalog: LocalMcpCatalog {
                    revision: Revision(0),
                    entries: vec![blender.clone()],
                },
            })
            .unwrap();
        store
            .save_project_local_mcps(
                project.id,
                SetProjectLocalMcps {
                    request_id: RequestId::new(),
                    settings: ProjectLocalMcps {
                        revision: Revision(0),
                        required: vec![],
                        optional: vec!["blender".into()],
                    },
                },
            )
            .unwrap();
        assert!(matches!(
            store.save_local_mcp_catalog(SetLocalMcpCatalog {
                request_id: RequestId::new(),
                catalog: LocalMcpCatalog {
                    revision: catalog.revision,
                    entries: vec![]
                },
            }),
            Err(StoreError::Invalid(_))
        ));

        let save = |ticket: Option<&Ticket>, token, draft, local_mcps| {
            store.submit_task_with_model(SubmitTaskWithModel {
                submission: SubmitTask {
                    edit_token: token,
                    draft,
                    files: vec![],
                    task: SaveTask {
                        request_id: RequestId::new(),
                        id: ticket.map(|t| t.id),
                        project_id: project.id,
                        expected_revision: ticket.map_or(Revision(0), |t| t.revision),
                        prompt: "Render".into(),
                        images: vec![],
                        requested_session: None,
                    },
                },
                implementer: None,
                local_mcps,
            })
        };
        assert!(matches!(
            save(None, None, false, Some(vec!["not-allowed".into()])),
            Err(StoreError::Invalid(_))
        ));
        let default_draft = save(None, None, true, None).unwrap();
        let empty_draft = save(None, None, true, Some(vec![])).unwrap();
        {
            let db = store.0.lock().unwrap();
            assert_eq!(task_optional(&db, default_draft.id).unwrap(), ["blender"]);
            assert_eq!(for_ticket(&db, default_draft.id, project.id).unwrap().local, [blender.clone()]);
            assert!(for_ticket(&db, empty_draft.id, project.id).unwrap().is_empty(),
                "An explicit deselection must not be replaced by the defaults");
        }
        let draft = save(None, None, true, Some(vec!["blender".into()])).unwrap();
        let edit = store
            .begin_task_edit(
                draft.id,
                BeginTaskEdit {
                    request_id: RequestId::new(),
                    expected_revision: draft.revision,
                    edit_token: None,
                },
            )
            .unwrap();
        assert_eq!(edit.local_mcps, ["blender"]);
        let task = save(Some(&edit.ticket), Some(edit.edit_token), false, None).unwrap();
        {
            let db = store.0.lock().unwrap();
            assert!(selected(&db, project.id).unwrap().is_empty());
            assert_eq!(
                for_ticket(&db, task.id, project.id).unwrap(),
                McpProfile {
                    local: vec![blender.clone()],
                    remote: vec![]
                }
            );
            assert!(!runner_ready(
                &db,
                runner,
                &McpProfile {
                    local: vec![blender.clone()],
                    remote: vec![]
                }
            )
            .unwrap());
        }
        store.autostart_runner(runner).unwrap();
        store.autostart_runner(another).unwrap();
        store.observe_mcp_runner(another, true, false).unwrap();
        store.scheduler_tick().unwrap();
        assert!(store
            .snapshot()
            .unwrap()
            .tickets
            .iter()
            .find(|t| t.id == task.id)
            .unwrap()
            .routing_note
            .contains("Wartet auf verfügbaren Runner"));
        assert!(store.pending(another).unwrap().is_empty());
        store.observe_mcp_runner(runner, true, false).unwrap();
        store.scheduler_tick().unwrap();
        let complete = |command: &RunnerCommand, result| {
            store
                .acknowledge(
                    command.runner_id,
                    command.id,
                    AckCommand {
                        request_id: RequestId::new(),
                        ownership_epoch: command.ownership_epoch,
                    },
                )
                .unwrap();
            store
                .complete(
                    command.runner_id,
                    command.id,
                    CommandResult {
                        request_id: RequestId::new(),
                        ownership_epoch: command.ownership_epoch,
                        result,
                    },
                )
                .unwrap();
        };
        let prepare = |command: &RunnerCommand| {
            complete(
                command,
                CommandOutput::Workspace {
                    workspace: Workspace {
                        path: format!("/fixture/{}", command.stream_id),
                        branch: command.workspace_spec.as_ref().unwrap().branch.clone(),
                        base_commit: "a".repeat(40),
                    },
                },
            );
        };
        // A prompt task is routed by a read-only agent before its worker starts.
        let controller = store
            .snapshot()
            .unwrap()
            .commands
            .into_iter()
            .find(|c| c.kind == CommandKind::PrepareWorkspace)
            .unwrap();
        prepare(&controller);
        store.scheduler_tick().unwrap();
        let routing = store
            .pending(controller.runner_id)
            .unwrap()
            .into_iter()
            .find(|c| c.agent_spec.as_ref().is_some_and(|a| a.role == "orchestrator"))
            .unwrap();
        assert!(routing.agent_spec.as_ref().unwrap().local_mcps.is_empty());
        complete(
            &routing,
            CommandOutput::Agent {
                agent: AgentProgress {
                    session_id: routing.session_id(),
                    message_id: agent_message_id(routing.id),
                    phase: AgentPhase::Succeeded,
                    summary: r#"{"action":"new","reason":"Start a fresh session"}"#.into(),
                },
            },
        );
        store.scheduler_tick().unwrap();
        let prep = store
            .pending(runner)
            .unwrap()
            .into_iter()
            .find(|c| c.kind == CommandKind::PrepareWorkspace)
            .unwrap();
        prepare(&prep);
        store.scheduler_tick().unwrap();
        let command = store
            .pending(runner)
            .unwrap()
            .into_iter()
            .find(|c| c.kind == CommandKind::RunAgent && c.stream_id == prep.stream_id)
            .unwrap();
        assert_eq!(
            command.agent_spec.as_ref().unwrap().local_mcps,
            [blender.clone()]
        );
        assert!(store
            .require_local_mcp_support(runner, command.id, command.ownership_epoch, false)
            .is_err());
        assert!(store.pending(another).unwrap().is_empty());
        drop(store);
        let store = Store::open(&path).unwrap();
        let db = store.0.lock().unwrap();
        let profile = McpProfile {
            local: vec![blender],
            remote: vec![],
        };
        assert!(!runner_ready(&db, runner, &profile).unwrap());
        assert_eq!(for_ticket(&db, task.id, project.id).unwrap(), profile);
    }
}
