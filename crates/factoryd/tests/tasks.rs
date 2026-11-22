use factory_core::*;
use factory_protocol::*;
use factoryd::store::Store;

#[test]
fn legacy_missing_folder_failure_gets_one_fenced_replacement_and_scheduler_continues() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let s = Store::open(&path).unwrap();
    let p = s
        .create_project(CreateProject {
            request_id: RequestId::new(),
            name: "Legacy".into(),
            repository: "/new/project".into(),
            base_branch: "main".into(),
        })
        .unwrap();
    s.create_ticket(CreateTicket {
        request_id: RequestId::new(),
        project_id: p.id,
        title: "Build it".into(),
        description: "Legacy task".into(),
        acceptance_criteria: String::new(),
        priority: 1,
    })
    .unwrap();
    let runner = RunnerId::new();
    s.autostart_runner(runner).unwrap();
    s.scheduler_tick().unwrap();
    let old = s.snapshot().unwrap().commands[0].clone();
    s.acknowledge(
        runner,
        old.id,
        AckCommand {
            request_id: RequestId::new(),
            ownership_epoch: 1,
        },
    )
    .unwrap();
    s.complete(runner,old.id,CommandResult{request_id:RequestId::new(),ownership_epoch:1,result:CommandOutput::WorkspaceError{workspace_error:"Repository import failed; check runner Git access. Retry preparation after fixing access; no files deleted".into()}}).unwrap();
    drop(s);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE local_project_recovery; PRAGMA user_version=23;")
        .unwrap();
    drop(db);
    let s = Store::open(&path).unwrap();
    s.recover_local_projects(RunnerId::new()).unwrap();
    assert_eq!(s.snapshot().unwrap().commands.len(), 1);
    let p = s.snapshot().unwrap().projects[0].clone();
    let p = s
        .pause_project(
            p.id,
            PauseProject {
                request_id: RequestId::new(),
                expected_revision: p.revision,
                paused: true,
            },
        )
        .unwrap();
    s.recover_local_projects(runner).unwrap();
    assert_eq!(s.snapshot().unwrap().commands.len(), 1);
    s.pause_project(
        p.id,
        PauseProject {
            request_id: RequestId::new(),
            expected_revision: p.revision,
            paused: false,
        },
    )
    .unwrap();
    s.recover_local_projects(runner).unwrap();
    s.recover_local_projects(runner).unwrap();
    let commands = s.snapshot().unwrap().commands;
    assert_eq!(commands.len(), 2);
    let new = commands.iter().find(|c| c.id != old.id).unwrap();
    assert_eq!(new.workspace_spec, old.workspace_spec);
    s.acknowledge(
        runner,
        new.id,
        AckCommand {
            request_id: RequestId::new(),
            ownership_epoch: 1,
        },
    )
    .unwrap();
    s.complete(
        runner,
        new.id,
        CommandResult {
            request_id: RequestId::new(),
            ownership_epoch: 1,
            result: CommandOutput::Workspace {
                workspace: Workspace {
                    path: "/work/recovered".into(),
                    branch: new.workspace_spec.as_ref().unwrap().branch.clone(),
                    base_commit: "a".repeat(40),
                },
            },
        },
    )
    .unwrap();
    drop(s);
    let s = Store::open(&path).unwrap();
    s.recover_local_projects(runner).unwrap();
    s.scheduler_tick().unwrap();
    s.scheduler_tick().unwrap();
    let commands = s.snapshot().unwrap().commands;
    assert_eq!(commands.len(), 3);
    assert_eq!(
        commands
            .iter()
            .filter(|c| c.kind == CommandKind::RunAgent)
            .count(),
        1
    );
    let agent = commands
        .iter()
        .find(|c| c.kind == CommandKind::RunAgent)
        .unwrap();
    s.acknowledge(
        runner,
        agent.id,
        AckCommand {
            request_id: RequestId::new(),
            ownership_epoch: 1,
        },
    )
    .unwrap();
    s.complete(
        runner,
        agent.id,
        CommandResult {
            request_id: RequestId::new(),
            ownership_epoch: 1,
            result: CommandOutput::Agent {
                agent: AgentProgress {
                    session_id: agent_session_id(agent.id),
                    message_id: agent_message_id(agent.id),
                    phase: AgentPhase::Succeeded,
                    summary: "Done".into(),
                },
            },
        },
    )
    .unwrap();
    let next = s
        .save_task(SaveTask {
            request_id: RequestId::new(),
            id: None,
            expected_revision: Revision(0),
            project_id: p.id,
            prompt: "Follow-up".into(),
            images: vec![],
        })
        .unwrap();
    s.scheduler_tick().unwrap();
    assert!(s
        .snapshot()
        .unwrap()
        .streams
        .iter()
        .any(|s| s.ticket_ids.contains(&next.id)));
}

#[test]
fn prompt_images_autostart_pause_restart_and_capability_fencing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let s = Store::open(&path).unwrap();
    let p = s
        .create_project(CreateProject {
            request_id: RequestId::new(),
            name: "Auto".into(),
            repository: "factory:managed".into(),
            base_branch: "main".into(),
        })
        .unwrap();
    let image = PromptImage::from_bytes("example.png".into(), b"\x89PNG\r\n\x1a\nfixture").unwrap();
    let b = SaveTask {
        request_id: RequestId::new(),
        id: None,
        expected_revision: Revision(0),
        project_id: p.id,
        prompt: "Implement this exactly\nwith tests".into(),
        images: vec![image.clone()],
    };
    let t = s.save_task(b.clone()).unwrap();
    assert_eq!(s.save_task(b.clone()).unwrap().id, t.id);
    let p = s
        .pause_project(
            p.id,
            PauseProject {
                request_id: RequestId::new(),
                expected_revision: p.revision,
                paused: true,
            },
        )
        .unwrap();
    let runner = RunnerId::new();
    s.autostart_runner(runner).unwrap();
    s.scheduler_tick().unwrap();
    assert!(s.snapshot().unwrap().commands.is_empty());
    let p = s
        .snapshot()
        .unwrap()
        .projects
        .into_iter()
        .find(|v| v.id == p.id)
        .unwrap();
    s.pause_project(
        p.id,
        PauseProject {
            request_id: RequestId::new(),
            expected_revision: p.revision,
            paused: false,
        },
    )
    .unwrap();
    s.scheduler_tick().unwrap();
    let c = s.snapshot().unwrap().commands[0].clone();
    let p = s.snapshot().unwrap().projects[0].clone();
    let p = s
        .pause_project(
            p.id,
            PauseProject {
                request_id: RequestId::new(),
                expected_revision: p.revision,
                paused: true,
            },
        )
        .unwrap();
    assert!(s.pending(runner).unwrap().is_empty());
    assert!(s
        .acknowledge(
            runner,
            c.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1
            }
        )
        .is_err());
    s.pause_project(
        p.id,
        PauseProject {
            request_id: RequestId::new(),
            expected_revision: p.revision,
            paused: false,
        },
    )
    .unwrap();
    s.acknowledge(
        runner,
        c.id,
        AckCommand {
            request_id: RequestId::new(),
            ownership_epoch: 1,
        },
    )
    .unwrap();
    s.complete(
        runner,
        c.id,
        CommandResult {
            request_id: RequestId::new(),
            ownership_epoch: 1,
            result: CommandOutput::Workspace {
                workspace: Workspace {
                    path: "/work/auto".into(),
                    branch: c.workspace_spec.unwrap().branch,
                    base_commit: "a".repeat(40),
                },
            },
        },
    )
    .unwrap();
    drop(s);
    let s = Store::open(&path).unwrap();
    s.scheduler_tick().unwrap();
    s.scheduler_tick().unwrap();
    let commands = s.snapshot().unwrap().commands;
    assert_eq!(commands.len(), 2);
    let c = commands
        .iter()
        .find(|c| c.kind == CommandKind::RunAgent)
        .unwrap();
    let spec = c.agent_spec.as_ref().unwrap();
    assert!(spec.prompt.starts_with(&b.prompt));
    assert_eq!(spec.images, vec![image]);
    assert!(s.require_image_support(runner, c.id, 1, false).is_err());
    assert!(s.require_image_support(runner, c.id, 1, true).is_ok());
    assert_eq!(
        s.scheduler_policy(p.id).unwrap().unwrap().remaining_starts,
        UNLIMITED_STARTS
    );
    s.acknowledge(
        runner,
        c.id,
        AckCommand {
            request_id: RequestId::new(),
            ownership_epoch: 1,
        },
    )
    .unwrap();
    s.complete(
        runner,
        c.id,
        CommandResult {
            request_id: RequestId::new(),
            ownership_epoch: 1,
            result: CommandOutput::Agent {
                agent: AgentProgress {
                    session_id: agent_session_id(c.id),
                    message_id: agent_message_id(c.id),
                    phase: AgentPhase::Succeeded,
                    summary: "result".into(),
                },
            },
        },
    )
    .unwrap();
    let mut next = b.clone();
    next.request_id = RequestId::new();
    next.prompt = "Next task".into();
    let next = s.save_task(next).unwrap();
    s.scheduler_tick().unwrap();
    assert!(s
        .snapshot()
        .unwrap()
        .streams
        .iter()
        .any(|s| s.ticket_ids.contains(&next.id)));
    let mut edit = b;
    edit.id = Some(t.id);
    edit.request_id = RequestId::new();
    edit.expected_revision = s.snapshot().unwrap().tickets[0].revision;
    assert!(s.save_task(edit).is_err());
}
