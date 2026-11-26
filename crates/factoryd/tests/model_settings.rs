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
            reviewer: Some(selection("c")),
            ..Default::default()
        },
        accesses: ["a", "b", "c"]
            .into_iter()
            .map(|name| ModelAccess {
                id: format!("factory-{name}"),
                name: name.into(),
                models: vec!["same-model".into()],
                transports: Default::default(),
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
            logo: None,
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
                        reviewer: Some(selection("a")),
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
            local_mcps: None,
        })
        .unwrap();
    store.autostart_runner(runner).unwrap();
    let models = store.task_models(ticket.id).unwrap().unwrap();
    assert_eq!(models.implementer, Some(selection("c")));
    assert_eq!(models.orchestrator, Some(selection("b")));
    assert_eq!(models.supervisor, Some(selection("c")));
    assert_eq!(models.memorizer, Some(selection("a")));
    assert_eq!(models.reviewer, Some(selection("a")));
    let mut next = settings(1, Some(selection("b")));
    next.roles.orchestrator = Some(selection("a"));
    next.roles.memorizer = Some(selection("c"));
    set(&store, next);
    let mut settings = store.project_models(project.id).unwrap();
    settings.roles.memorizer = None;
    settings.roles.reviewer = None;
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
        local_mcps: None,
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
    let reviewer = store.pending(runner).unwrap().pop().unwrap();
    let spec = reviewer.agent_spec.as_ref().unwrap();
    assert_eq!(spec.role, "reviewer");
    assert_eq!(spec.profile.provider_id, "factory-a", "Use the pinned project Reviewer model despite later Settings changes");
    complete(&store, &reviewer, CommandOutput::Agent { agent: AgentProgress {
        session_id: reviewer.session_id(), message_id: agent_message_id(reviewer.id),
        phase: AgentPhase::Succeeded, summary: r#"{"action":"approve"}"#.into(),
    }});
    store.scheduler_tick().unwrap();
    let pending = store.pending(runner).unwrap();
    assert_eq!(pending.len(), 1);
    let spec = pending[0].agent_spec.as_ref().unwrap();
    assert_eq!(spec.role, "memorizer");
    assert_eq!(spec.profile.provider_id, "factory-a");
    let prompt = PromptSession {
        request_id: RequestId::new(), prompt: "Continue directly with this session's settings".into(),
        images: vec![], files: vec![],
    };
    let follow = store.prompt_session(work.id, prompt.clone()).unwrap();
    let pins = store.task_models(follow.id).unwrap().unwrap();
    assert_eq!(pins.implementer, Some(selection("c")), "Inherit session access, not the changed default");
    assert!(pins.orchestrator.is_none(), "User-directed prompts bypass the orchestrator");
    let owner = store.snapshot().unwrap().streams.into_iter().find(|s| s.id == work.stream_id).unwrap();
    store.close_session(work.id, CloseSession {
        request_id: RequestId::new(), expected_stream_revision: owner.revision, reason: "Close test session".into(),
    }).unwrap();
    assert!(store.session_closure(work.id).unwrap().is_some());
    let new_skill = store.import_factory_skill(factory_protocol::skills::ImportFactorySkill {
        request_id: RequestId::new(), id: "review".into(), name: "Review".into(),
        description: "Review checklist".into(), current_version: 0,
        files: [("SKILL.md".into(), "# Review".into())].into(),
    }).unwrap();
    store.set_project_skills(project.id, SetProjectSkills {
        request_id: RequestId::new(), expected_revision: Revision(0),
        repo_skills: vec![], factory_skills: vec![new_skill.reference],
    }).unwrap();
    assert!(store.prompt_session(work.id, PromptSession {
        request_id: RequestId::new(), prompt: String::new(), images: vec![], files: vec![],
    }).is_err());
    assert!(store.session_closure(work.id).unwrap().is_some(), "Rejected prompts must not reopen the session");
    assert_eq!(store.prompt_session(work.id, prompt.clone()).unwrap().id, follow.id);
    assert!(store.session_closure(work.id).unwrap().is_some(), "Replaying an old request is not a new reactivation");
    let reactivation = PromptSession { request_id: RequestId::new(), ..prompt };
    let reopened = store.prompt_session(work.id, reactivation.clone()).unwrap();
    assert_ne!(reopened.id, follow.id);
    assert_eq!(reopened.requested_session, Some(work.session_root()));
    assert!(store.session_closure(work.id).unwrap().is_none());
    assert_eq!(store.prompt_session(work.id, reactivation).unwrap().id, reopened.id);
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
    assert_eq!(pin.reviewer, Some(selection("a")));
    let new = store.save_task(submit(p.id, "Uses new default")).unwrap();
    assert_eq!(store.task_models(new.id).unwrap().unwrap().memorizer, Some(selection("b")));
    assert_eq!(store.task_models(new.id).unwrap().unwrap().reviewer, Some(selection("c")));
    assert_eq!(
        store.task_models(new.id).unwrap().unwrap().implementer,
        Some(selection("b"))
    );
}
