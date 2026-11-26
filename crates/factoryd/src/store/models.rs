use super::*;
use factory_protocol::models::*;
use std::collections::HashSet;

pub(super) fn factory(db: &Connection) -> Result<Option<FactoryModelSettings>> {
    let data: Option<String> = db
        .query_row(
            "SELECT data FROM factory_model_settings WHERE id=1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    data.map(|raw| Ok(serde_json::from_str(&raw)?)).transpose()
}

fn project(db: &Connection, id: ProjectId) -> Result<ProjectModelSettings> {
    let data: Option<String> = db
        .query_row(
            "SELECT data FROM project_model_settings WHERE project_id=?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(data
        .map(|raw| serde_json::from_str(&raw))
        .transpose()?
        .unwrap_or(ProjectModelSettings {
            revision: Revision(0),
            roles: RoleModels::default(),
        }))
}

pub(super) fn ticket(db: &Connection, id: TicketId) -> Result<Option<TaskModels>> {
    let data: Option<String> = db
        .query_row(
            "SELECT data FROM task_models WHERE ticket_id=?1",
            [id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    data.map(|raw| Ok(serde_json::from_str(&raw)?)).transpose()
}

pub(super) fn for_command(db: &Connection, command: &RunnerCommand) -> Result<Option<TaskModels>> {
    let data: Option<String> = db
        .query_row(
            "SELECT m.data FROM stream_tickets st JOIN tickets t ON t.id=st.ticket_id
         JOIN task_models m ON m.ticket_id=t.id WHERE st.stream_id=?1
         AND json_extract(t.data,'$.execution_command')=?2 LIMIT 1",
            params![command.stream_id.to_string(), command.id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    data.map(|raw| Ok(serde_json::from_str(&raw)?)).transpose()
}

fn verify_project(db: &Connection, id: ProjectId) -> Result<()> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM active_projects WHERE id=?1)",
        [id.to_string()],
        |r| r.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::NotFound)
    }
}

fn check_selection(catalog: &FactoryModelSettings, selection: &ModelSelection) -> Result<()> {
    if catalog
        .accesses
        .iter()
        .any(|a| a.id == selection.access_id && a.models.contains(&selection.model_id))
    {
        Ok(())
    } else {
        Err(StoreError::Invalid(
            "Der gewählte Modellzugang oder das Modell ist nicht im Factory-Katalog",
        ))
    }
}

fn check_roles(catalog: &FactoryModelSettings, roles: &RoleModels) -> Result<()> {
    for model in [&roles.orchestrator, &roles.implementer, &roles.supervisor, &roles.memorizer, &roles.merger, &roles.reviewer]
        .into_iter()
        .flatten()
    {
        check_selection(catalog, model)?;
    }
    Ok(())
}

fn clear_role_accesses(roles: &mut RoleModels, alias: &str) -> bool {
    let mut changed = false;
    for model in [&mut roles.orchestrator, &mut roles.implementer, &mut roles.supervisor, &mut roles.memorizer, &mut roles.merger, &mut roles.reviewer] {
        if model.as_ref().is_some_and(|m| m.access_id == alias) {
            *model = None;
            changed = true;
        }
    }
    changed
}

fn role(roles: &RoleModels, role: &str) -> Option<ModelSelection> {
    match role {
        "orchestrator" | "orchestrator-question" | "orchestrator-memory" => {
            roles.orchestrator.clone()
        }
        "supervisor" => roles.supervisor.clone(),
        "reviewer" => roles.reviewer.clone(),
        "memorizer" => roles.memorizer.clone(),
        MERGER_ROLE => roles.merger.clone(),
        _ => roles.implementer.clone(),
    }
}

fn resolve(
    factory: &FactoryModelSettings,
    project: &RoleModels,
    name: &str,
) -> Option<ModelSelection> {
    role(project, name)
        .or_else(|| role(&factory.roles, name))
        .or_else(|| factory.default_model.clone())
}

pub(super) fn role_selection(
    db: &Connection,
    id: ProjectId,
    name: &str,
) -> Result<Option<ModelSelection>> {
    let Some(factory) = factory(db)? else {
        return Ok(None);
    };
    if factory.default_model.is_none() {
        return Err(StoreError::Invalid(
            "OpenCode zuerst in der Factory einrichten",
        ));
    }
    Ok(resolve(&factory, &project(db, id)?.roles, name))
}

pub(super) fn apply(profile: &mut ExecutionProfile, selection: &ModelSelection) {
    profile.provider_id = selection.access_id.clone();
    profile.model_id = selection.model_id.clone();
}

pub(super) fn for_task(
    db: &Connection,
    id: TicketId,
    project_id: ProjectId,
    choice: Option<&Option<ModelSelection>>,
    draft: bool,
) -> Result<Option<TaskModels>> {
    let previous = ticket(db, id)?;
    let chosen = choice.cloned().unwrap_or_else(|| {
        previous
            .as_ref()
            .and_then(|m| m.implementer_override.clone())
    });
    let Some(factory) = factory(db)? else {
        if chosen.is_some() {
            return Err(StoreError::Invalid(
                "OpenCode zuerst in der Factory einrichten",
            ));
        }
        return Ok(None);
    };
    if let Some(model) = &chosen {
        check_selection(&factory, model)?;
    }
    if draft {
        return Ok(Some(TaskModels {
            implementer_override: chosen,
            orchestrator: None,
            implementer: None,
            reviewer: None,
            supervisor: None,
            memorizer: None,
        }));
    }
    if factory.default_model.is_none() {
        return Err(StoreError::Invalid(
            "OpenCode zuerst in der Factory einrichten",
        ));
    }
    let roles = project(db, project_id)?.roles;
    let required = |selection: Option<ModelSelection>| {
        selection.ok_or(StoreError::Invalid(
            "OpenCode zuerst in der Factory einrichten",
        ))
    };
    let orchestrator = required(resolve(&factory, &roles, "orchestrator"))?;
    let implementer = required(
        chosen
            .clone()
            .or_else(|| resolve(&factory, &roles, "implementer")),
    )?;
    let supervisor = required(resolve(&factory, &roles, "supervisor"))?;
    let reviewer = required(resolve(&factory, &roles, "reviewer"))?;
    let memorizer = required(resolve(&factory, &roles, "memorizer"))?;
    check_selection(&factory, &orchestrator)?;
    check_selection(&factory, &implementer)?;
    check_selection(&factory, &supervisor)?;
    check_selection(&factory, &reviewer)?;
    check_selection(&factory, &memorizer)?;
    Ok(Some(TaskModels {
        implementer_override: chosen,
        orchestrator: Some(orchestrator),
        implementer: Some(implementer),
        reviewer: Some(reviewer),
        supervisor: Some(supervisor),
        memorizer: Some(memorizer),
    }))
}

pub(super) fn save_ticket(db: &Connection, id: TicketId, models: &TaskModels) -> Result<()> {
    db.execute("INSERT INTO task_models(ticket_id,data) VALUES(?1,?2) ON CONFLICT(ticket_id) DO UPDATE SET data=excluded.data",
        params![id.to_string(), serde_json::to_string(models)?])?;
    Ok(())
}

pub(super) fn require_available(db: &Connection, selection: &ModelSelection) -> Result<()> {
    let catalog = factory(db)?.ok_or(StoreError::Invalid(
        "OpenCode zuerst in der Factory einrichten",
    ))?;
    check_selection(&catalog, selection)
}

fn pin_legacy_tasks(tx: &Transaction<'_>, default: &ModelSelection) -> Result<()> {
    let pin = TaskModels {
        implementer_override: None,
        orchestrator: Some(default.clone()),
        implementer: Some(default.clone()),
        reviewer: Some(default.clone()),
        supervisor: Some(default.clone()),
        memorizer: Some(default.clone()),
    };
    tx.execute(
        "INSERT OR IGNORE INTO task_models(ticket_id,data)
        SELECT t.id,?1 FROM tickets t JOIN active_projects p ON p.id=t.project_id
        WHERE json_extract(t.data,'$.prompt') IS NOT NULL
        AND json_extract(t.data,'$.status') IN ('backlog','ready')
        AND COALESCE(json_extract(t.data,'$.draft'),0)=0
        AND json_extract(t.data,'$.execution_command') IS NULL
        AND NOT EXISTS(SELECT 1 FROM task_routing e WHERE e.ticket_id=t.id)",
        [serde_json::to_string(&pin)?],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_removal_clears_project_roles_but_protects_open_intents() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        store.enable_model_settings().unwrap();
        let access = ModelAccess {
            id: "factory-removal-test".into(), name: "Test".into(),
            models: vec!["model".into()], transports: Default::default(),
        };
        let model = store.add_connected_model_access(access.clone(), "test-credential").unwrap().unwrap();
        let project = store.create_project(CreateProject {
            logo: None,
            request_id: RequestId::new(), runner_mapping: None, key: None,
            name: "Models".into(), repository: "factory:managed".into(), base_branch: "main".into(),
        }).unwrap();
        store.set_project_models(project.id, SetProjectModels {
            request_id: RequestId::new(),
            settings: ProjectModelSettings {
                revision: Revision(0),
                roles: RoleModels { implementer: Some(model), ..Default::default() },
            },
        }).unwrap();
        let task = store.save_task(SaveTask {
            request_id: RequestId::new(), id: None, expected_revision: Revision(0),
            project_id: project.id, prompt: "Test access pin".into(),
            requested_session: None, images: vec![],
        }).unwrap();
        assert!(matches!(store.remove_model_access(&access.id), Err(StoreError::Invalid(_))));
        {
            let db = store.0.lock().unwrap();
            db.execute("UPDATE tickets SET data=json_set(data,'$.status','done') WHERE id=?1",
                [task.id.to_string()]).unwrap();
        }
        let (settings, credential) = store.remove_model_access(&access.id).unwrap();
        assert!(settings.accesses.is_empty() && settings.default_model.is_none());
        assert!(store.project_models(project.id).unwrap().roles.implementer.is_none());
        assert_eq!(credential.as_deref(), Some("test-credential"));
        assert!(store.task_models(task.id).unwrap().unwrap().implementer.is_some());
    }

    #[test]
    fn only_first_connected_access_sets_a_default_and_roles_inherit_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("factory.sqlite");
        let store = Store::open(&path).unwrap();
        store.enable_model_settings().unwrap();
        let access = |id: &str, model: &str| ModelAccess {
            id: format!("factory-{id}"),
            name: id.into(),
            models: vec![model.into()],
            transports: Default::default(),
        };
        let first = store
            .add_connected_model_access(access("work", "first-model"), "credential-1")
            .unwrap()
            .expect("first access auto-selects a model");
        assert_eq!(first.access_id, "factory-work");
        assert_eq!(first.model_id, "first-model");
        assert!(store
            .add_connected_model_access(access("personal", "other-model"), "credential-2")
            .unwrap()
            .is_none());
        drop(store);
        let settings = Store::open(&path).unwrap().factory_models().unwrap();
        assert_eq!(settings.default_model, Some(first.clone()));
        for role in ["orchestrator", "implementer", "reviewer", "supervisor", "memorizer"] {
            assert_eq!(
                resolve(&settings, &RoleModels::default(), role),
                Some(first.clone())
            );
        }
    }
}

impl Store {
    pub(crate) fn remove_model_access(&self, alias: &str) -> Result<(FactoryModelSettings, Option<String>)> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut settings = factory(&tx)?.ok_or(StoreError::NotFound)?;
        if !settings.accesses.iter().any(|access| access.id == alias) {
            return Ok((settings, None));
        }
        let in_use: bool = tx.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM task_models m
                JOIN tickets t ON t.id=m.ticket_id
                JOIN active_projects p ON p.id=t.project_id
                WHERE json_extract(t.data,'$.status') NOT IN ('done','cancelled')
                AND EXISTS (SELECT 1 FROM json_each(m.data) role
                            WHERE json_extract(role.value,'$.access_id')=?1)
             )",
            [alias],
            |row| row.get(0),
        )?;
        if in_use {
            return Err(StoreError::Invalid("Dieser Modellzugang wird noch von offenen Intents verwendet."));
        }
        settings.accesses.retain(|access| access.id != alias);
        if settings.default_model.as_ref().is_some_and(|model| model.access_id == alias) {
            settings.default_model = None;
        }
        clear_role_accesses(&mut settings.roles, alias);
        settings.revision.0 = settings.revision.0.checked_add(1).ok_or(StoreError::Internal)?;
        tx.execute(
            "UPDATE factory_model_settings SET data=?1 WHERE id=1",
            [serde_json::to_string(&settings)?],
        )?;
        let projects = {
            let mut query = tx.prepare("SELECT project_id,data FROM project_model_settings")?;
            let rows = query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, raw) in projects {
            let mut project: ProjectModelSettings = serde_json::from_str(&raw)?;
            if clear_role_accesses(&mut project.roles, alias) {
                project.revision.0 = project.revision.0.checked_add(1).ok_or(StoreError::Internal)?;
                tx.execute(
                    "UPDATE project_model_settings SET data=?2 WHERE project_id=?1",
                    params![id, serde_json::to_string(&project)?],
                )?;
                event(&tx, "models.project_changed", id, &project)?;
            }
        }
        let credential = tx.query_row(
            "DELETE FROM model_access_bindings WHERE access_id=?1 RETURNING credential_id",
            [alias],
            |row| row.get(0),
        ).optional()?;
        event(&tx, "models.factory_changed", "factory".into(), &settings)?;
        tx.commit()?;
        Ok((settings, credential))
    }

    pub(crate) fn add_connected_model_access(
        &self,
        access: ModelAccess,
        credential: &str,
    ) -> Result<Option<ModelSelection>> {
        self.add_model_access(access, Some(credential))
    }

    pub(crate) fn add_opencode_free_access(&self, access: ModelAccess) -> Result<Option<ModelSelection>> {
        debug_assert_eq!(access.id, OPENCODE_FREE_ACCESS);
        self.add_model_access(access, None)
    }

    fn add_model_access(
        &self,
        access: ModelAccess,
        credential: Option<&str>,
    ) -> Result<Option<ModelSelection>> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut settings = factory(&tx)?.ok_or(StoreError::Internal)?;
        if settings.accesses.len() >= 64
            || settings.accesses.iter().any(|a| {
                a.id == access.id || a.name.trim().to_lowercase() == access.name.to_lowercase()
            })
        {
            return Err(StoreError::WorkConflict);
        }
        let already_connected = settings.accesses.iter().any(|a| a.id == OPENCODE_FREE_ACCESS)
            || tx.query_row("SELECT EXISTS(SELECT 1 FROM model_access_bindings)", [], |r| r.get::<_, bool>(0))?;
        let automatic_default = if !already_connected && settings.default_model.is_none() {
            access.models.first().map(|model_id| ModelSelection {
                access_id: access.id.clone(),
                model_id: model_id.clone(),
            })
        } else {
            None
        };
        if let Some(default) = &automatic_default {
            settings.default_model = Some(default.clone());
            pin_legacy_tasks(&tx, default)?;
        }
        if let Some(credential) = credential {
            tx.execute(
                "INSERT INTO model_access_bindings(access_id,credential_id) VALUES(?1,?2)",
                params![access.id, credential],
            )?;
        }
        settings.revision.0 = settings
            .revision
            .0
            .checked_add(1)
            .ok_or(StoreError::Internal)?;
        event(&tx, "models.access_bound", access.id.clone(), &access)?;
        settings.accesses.push(access);
        if automatic_default.is_some() {
            event(&tx, "models.factory_changed", "factory".into(), &settings)?;
        }
        tx.execute(
            "UPDATE factory_model_settings SET data=?1 WHERE id=1",
            [serde_json::to_string(&settings)?],
        )?;
        tx.commit()?;
        Ok(automatic_default)
    }
    pub fn enable_model_settings(&self) -> Result<()> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        db.execute(
            "INSERT OR IGNORE INTO factory_model_settings(id,data) VALUES(1,?1)",
            [serde_json::to_string(&FactoryModelSettings {
                revision: Revision(0),
                default_model: None,
                roles: RoleModels::default(),
                accesses: vec![],
            })?],
        )?;
        Ok(())
    }
    pub(crate) fn model_binding(&self, alias: &str) -> Result<Option<String>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        Ok(db
            .query_row(
                "SELECT credential_id FROM model_access_bindings WHERE access_id=?1",
                [alias],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn model_bindings(&self) -> Result<HashSet<String>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let mut statement = db.prepare("SELECT access_id FROM model_access_bindings")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn bind_model_access(
        &self,
        alias: &str,
        credential: &str,
        previous: Option<&str>,
    ) -> Result<()> {
        let mut db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !factory(&tx)?
            .is_some_and(|catalog| catalog.accesses.iter().any(|access| access.id == alias))
        {
            return Err(StoreError::NotFound);
        }
        let changed = if let Some(previous) = previous {
            tx.execute("UPDATE OR IGNORE model_access_bindings SET credential_id=?2 WHERE access_id=?1 AND credential_id=?3",
                params![alias, credential, previous])?
        } else {
            tx.execute("INSERT OR IGNORE INTO model_access_bindings(access_id,credential_id) VALUES(?1,?2)",
                params![alias, credential])?
        };
        if changed != 1 {
            return Err(StoreError::WorkConflict);
        }
        event(
            &tx,
            "models.access_bound",
            alias.into(),
            &serde_json::json!({"access_id":alias}),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn factory_models(&self) -> Result<FactoryModelSettings> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        Ok(factory(&db)?.unwrap_or(FactoryModelSettings {
            revision: Revision(0),
            default_model: None,
            roles: RoleModels::default(),
            accesses: vec![],
        }))
    }

    pub fn set_factory_models(&self, body: SetFactoryModels) -> Result<FactoryModelSettings> {
        self.mutate(body.request_id, "models.factory", &body, |tx| {
            let previous = factory(tx)?;
            let mut next = body.settings.clone();
            let revision = previous.as_ref().map_or(Revision(0), |v| v.revision);
            if next.revision != revision { return Err(StoreError::RevisionConflict(revision)); }
            let mut ids = HashSet::new();
            let mut names = HashSet::new();
            let free_access = |settings: &FactoryModelSettings| {
                settings.accesses.iter().find(|a| a.id == OPENCODE_FREE_ACCESS).cloned()
            };
            if free_access(&next) != previous.as_ref().and_then(free_access) {
                return Err(StoreError::Invalid("Den kostenlosen OpenCode-Zugang über Modell-Anbieter verwalten."));
            }
            if next.accesses.len() > 64 { return Err(StoreError::Invalid("Zu viele Modellzugänge")); }
            for access in &next.accesses {
                if !(access.id == OPENCODE_FREE_ACCESS || access.id.strip_prefix("factory-").is_some_and(|id|
                    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')))
                    || !ids.insert(&access.id) || !names.insert(access.name.trim().to_lowercase())
                    || !super::super::valid_text(&access.name, 120) || access.models.len() > 256
                {
                    return Err(StoreError::Invalid("Ungültiger oder doppelter Modellzugang"));
                }
                let mut models = HashSet::new();
                if access.models.iter().any(|m| !super::super::valid_text(m, 200) || !models.insert(m)) {
                    return Err(StoreError::Invalid("Ungültige oder doppelte Modell-ID"));
                }
            }
            if let Some(model) = &next.default_model { check_selection(&next, model)?; }
            check_roles(&next, &next.roles)?;
            next.revision.0 = revision.0.checked_add(1).ok_or(StoreError::Internal)?;
            tx.execute("INSERT INTO factory_model_settings(id,data) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
                [serde_json::to_string(&next)?])?;
            // Unassigned legacy tasks used to be admitted without an explicit model.
            // Pin the first available default once; never rewrite an existing pin.
            if let Some(default) = &next.default_model {
                if previous.as_ref().is_none_or(|v| v.default_model.is_none()) {
                    pin_legacy_tasks(tx, default)?;
                }
            }
            event(tx, "models.factory_changed", "factory".into(), &next)?;
            Ok(next)
        })
    }

    pub fn project_models(&self, id: ProjectId) -> Result<ProjectModelSettings> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        verify_project(&db, id)?;
        project(&db, id)
    }

    pub fn set_project_models(
        &self,
        id: ProjectId,
        body: SetProjectModels,
    ) -> Result<ProjectModelSettings> {
        self.mutate(
            body.request_id,
            &format!("models.project/{id}"),
            &body,
            |tx| {
                verify_project(tx, id)?;
                let original = project(tx, id)?;
                let mut next = body.settings.clone();
                if next.revision != original.revision {
                    return Err(StoreError::RevisionConflict(original.revision));
                }
                let factory = factory(tx)?.ok_or(StoreError::Invalid(
                    "OpenCode zuerst in der Factory einrichten",
                ))?;
                check_roles(&factory, &next.roles)?;
                next.revision.0 = original
                    .revision
                    .0
                    .checked_add(1)
                    .ok_or(StoreError::Internal)?;
                tx.execute(
                    "INSERT INTO project_model_settings(project_id,data) VALUES(?1,?2)
                ON CONFLICT(project_id) DO UPDATE SET data=excluded.data",
                    params![id.to_string(), serde_json::to_string(&next)?],
                )?;
                event(tx, "models.project_changed", id.to_string(), &next)?;
                Ok(next)
            },
        )
    }

    pub fn task_models(&self, id: TicketId) -> Result<Option<TaskModels>> {
        let db = self.0.lock().map_err(|_| StoreError::Internal)?;
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM tickets WHERE id=?1)",
            [id.to_string()],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound);
        }
        ticket(&db, id)
    }
}
