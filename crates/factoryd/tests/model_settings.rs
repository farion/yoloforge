use factory_core::*;
use factory_protocol::{models::*, *};
use factoryd::store::{Store, StoreError};

fn selection(access: &str) -> ModelSelection {
    ModelSelection {
        access_id: format!("factory-{access}"),
        model_id: "same-model".into(),
    }
}

fn settings(revision: u64, default: Option<ModelSelection>) -> FactoryModelSettings {
    FactoryModelSettings {
        revision: Revision(revision),
        default_model: default,
        roles: RoleModels {
            orchestrator: Some(selection("b")),
            memorizer: Some(selection("b")),
            ..Default::default()
        },
        accesses: ["a", "b", "c"]
            .into_iter()
            .map(|name| ModelAccess {
                id: format!("factory-{name}"),
                name: name.into(),
                models: vec!["same-model".into()],
            })
            .collect(),
    }
}

fn submit(project_id: ProjectId, prompt: &str) -> SaveTask {
    SaveTask {
        request_id: RequestId::new(),
        id: None,
        expected_revision: Revision(0),
        project_id,
        prompt: prompt.into(),
        images: vec![],
        requested_session: None,
    }
}

fn project(store: &Store) -> Project {
    store
        .create_project(CreateProject {
            request_id: RequestId::new(),
            runner_mapping: None,
            key: None,
            name: "Models".into(),
            repository: "factory:managed".into(),
            base_branch: "main".into(),
        })
        .unwrap()
}

fn set(store: &Store, next: FactoryModelSettings) {
    store
        .set_factory_models(SetFactoryModels {
            request_id: RequestId::new(),
            settings: next,
        })
        .unwrap();
}

fn complete(store: &Store, c: &RunnerCommand, result: CommandOutput) {
    store
        .acknowledge(
            c.runner_id,
            c.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: c.ownership_epoch,
            },
        )
        .unwrap();
    store
        .complete(
            c.runner_id,
            c.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: c.ownership_epoch,
                result,
            },
        )
        .unwrap();
}

fn prepare(store: &Store, c: &RunnerCommand) {
    assert_eq!(c.kind, CommandKind::PrepareWorkspace);
    complete(
        store,
        c,
        CommandOutput::Workspace {
            workspace: Workspace {
                path: format!("/work/{}", c.stream_id),
                branch: c.workspace_spec.as_ref().unwrap().branch.clone(),
                base_commit: "a".repeat(40),
            },
        },
    );
}

#[test]
fn role_precedence_and_ticket_override_are_frozen_and_used_for_real_command_enqueue() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("factory.db");
    let store = Store::open(&db).unwrap();
    let project = project(&store);
    set(&store, settings(0, Some(selection("a"))));
    store
        .set_project_models(
            project.id,
            SetProjectModels {
                request_id: RequestId::new(),
                settings: ProjectModelSettings {
                    revision: Revision(0),
                    roles: RoleModels {
                        implementer: Some(selection("b")),
                        supervisor: Some(selection("c")),
                        memorizer: Some(selection("a")),
                        ..Default::default()
                    },
                },
            },
        )
        .unwrap();
    let runner = RunnerId::new();
    let ticket = store
        .submit_task_with_model(SubmitTaskWithModel {
            submission: SubmitTask {
                task: submit(project.id, "Write tests"),
                edit_token: None,
                draft: false,
                files: vec![],
            },
            implementer: Some(selection("c")),
        })
        .unwrap();
    store.autostart_runner(runner).unwrap();
    let models = store.task_models(ticket.id).unwrap().unwrap();
    assert_eq!(models.implementer, Some(selection("c")));
    assert_eq!(models.orchestrator, Some(selection("b")));
    assert_eq!(models.supervisor, Some(selection("c")));
    assert_eq!(models.memorizer, Some(selection("a")));
    let mut next = settings(1, Some(selection("b")));
    next.roles.orchestrator = Some(selection("a"));
    next.roles.memorizer = Some(selection("c"));
    set(&store, next);
    let mut settings = store.project_models(project.id).unwrap();
    settings.roles.memorizer = None;
    store.set_project_models(project.id, SetProjectModels {
        request_id: RequestId::new(),
        settings,
    }).unwrap();
    drop(store);
    let store = Store::open(&db).unwrap();
    assert_eq!(store.task_models(ticket.id).unwrap(), Some(models));
    store.scheduler_tick().unwrap();
    let workspace = store.pending(runner).unwrap().pop().unwrap();
    prepare(&store, &workspace);
    store.scheduler_tick().unwrap();
    let route = store.pending(runner).unwrap().pop().unwrap();
    assert_eq!(
        route.agent_spec.as_ref().unwrap().profile.provider_id,
        "factory-b"
    );
    complete(
        &store,
        &route,
        CommandOutput::Agent {
            agent: AgentProgress {
                session_id: route.session_id(),
                message_id: agent_message_id(route.id),
                phase: AgentPhase::Succeeded,
                summary: r#"{"action":"new","reason":"Test"}"#.into(),
            },
        },
    );
    store.scheduler_tick().unwrap();
    prepare(&store, &store.pending(runner).unwrap().pop().unwrap());
    store.scheduler_tick().unwrap();
    let work = store
        .pending(runner)
        .unwrap()
        .into_iter()
        .find(|c| c.agent_spec.as_ref().is_some_and(|a| !a.read_only))
        .unwrap();
    assert_eq!(work.agent_spec.as_ref().unwrap().profile.provider_id, "factory-c");
    let mut fixed = submit(project.id, "Continue with incompatible access");
    fixed.requested_session = Some(work.id);
    let result = store.submit_task_with_model(SubmitTaskWithModel {
        submission: SubmitTask {
            task: fixed,
            edit_token: None,
            draft: false,
            files: vec![],
        },
        implementer: Some(selection("a")),
    });
    assert!(
        matches!(result, Err(StoreError::Invalid(message)) if message.contains("neue Session"))
    );
    complete(&store, &work, CommandOutput::Agent {
        agent: AgentProgress {
            session_id: work.session_id(),
            message_id: agent_message_id(work.id),
            phase: AgentPhase::Succeeded,
            summary: "Work completed".into(),
        },
    });
    store.report_archive(runner, work.id, ReportArchive {
        ownership_epoch: work.ownership_epoch,
        batch: ArchiveBatch {
            session_id: work.session_id(),
            revision: 1,
            captured_unix_ms: 1,
            final_scan_complete: true,
            messages: vec![],
        },
    }).unwrap();
    store.scheduler_tick().unwrap();
    let pending = store.pending(runner).unwrap();
    assert_eq!(pending.len(), 1);
    let spec = pending[0].agent_spec.as_ref().unwrap();
    assert_eq!(spec.role, "memorizer");
    assert_eq!(spec.profile.provider_id, "factory-a");
}

#[test]
fn format_migration_backfills_old_queue_once_and_missing_default_blocks_new_release() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("factory.db")).unwrap();
    let p = project(&store);
    let old = store
        .save_task(submit(p.id, "Enqueued before a model existed"))
        .unwrap();
    assert_eq!(store.task_models(old.id).unwrap(), None);
    store.enable_model_settings().unwrap();
    assert!(matches!(
        store.save_task(submit(p.id, "Must wait for setup")),
        Err(StoreError::Invalid(_))
    ));
    set(&store, settings(0, Some(selection("a"))));
    set(&store, settings(1, Some(selection("b"))));
    let pin = store.task_models(old.id).unwrap().unwrap();
    assert_eq!(pin.implementer, Some(selection("a")));
    assert_eq!(pin.orchestrator, Some(selection("a")));
    assert_eq!(pin.memorizer, Some(selection("a")));
    let new = store.save_task(submit(p.id, "Uses new default")).unwrap();
    assert_eq!(store.task_models(new.id).unwrap().unwrap().memorizer, Some(selection("b")));
    assert_eq!(
        store.task_models(new.id).unwrap().unwrap().implementer,
        Some(selection("b"))
    );
}
