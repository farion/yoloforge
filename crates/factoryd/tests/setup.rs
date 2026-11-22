use factory_core::*;
use factory_protocol::*;
use factoryd::store::Store;
#[test]
fn project_setup_is_atomic_replayable_and_pins_host_and_harness() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let s = Store::open(&path).unwrap();
    let b = SetupProject {
        request_id: RequestId::new(),
        project_id: None,
        expected_revision: Revision(0),
        name: "Wizard".into(),
        repository: "/repo".into(),
        base_branch: "main".into(),
        profile: ExecutionProfile {
            provider_id: "openai".into(),
            model_id: "test".into(),
            ..Default::default()
        },
        setup: ProjectSetup {
            repository_source: "/repo".into(),
            runner_id: RunnerId::new(),
            repository_path: "/repo".into(),
            service_registration: "".into(),
            harness: Some(serde_json::json!({"mcp":{"servers":{}},"instructions":["AGENTS.md"]})),
        },
    };
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER setup_failure BEFORE INSERT ON commands BEGIN SELECT RAISE(ABORT,'test'); END").unwrap();
    assert!(s.setup_project(b.clone()).is_err());
    assert!(s.snapshot().unwrap().projects.is_empty());
    db.execute_batch("DROP TRIGGER setup_failure").unwrap();
    let first = s.setup_project(b.clone()).unwrap();
    assert_eq!(
        first
            .command
            .workspace_spec
            .as_ref()
            .unwrap()
            .setup
            .as_ref(),
        Some(&b.setup)
    );
    assert!(s
        .require_setup_support(b.setup.runner_id, first.command.id, 1, false)
        .is_err());
    drop(s);
    let s = Store::open(&path).unwrap();
    let replay = s.setup_project(b.clone()).unwrap();
    assert_eq!(first.project.id, replay.project.id);
    assert_eq!(first.command.id, replay.command.id);
    let mut update = b.clone();
    update.request_id = RequestId::new();
    update.project_id = Some(first.project.id);
    update.expected_revision = first.project.revision;
    update.setup.harness = Some(serde_json::json!({"instructions":["OTHER.md"]}));
    update.setup.runner_id = RunnerId::new();
    update.setup.repository_path = "/remote/repo".into();
    let changed = s.setup_project(update.clone()).unwrap();
    assert_eq!(changed.project.runner_bindings.len(), 2);
    assert_eq!(
        changed.project.runner_bindings[&b.setup.runner_id].repository_path,
        "/repo"
    );
    update.request_id = RequestId::new();
    assert!(s.setup_project(update).is_err());
    let snapshot = s.snapshot().unwrap();
    assert_eq!(
        snapshot
            .commands
            .iter()
            .find(|c| c.id == first.command.id)
            .unwrap()
            .workspace_spec
            .as_ref()
            .unwrap()
            .setup
            .as_ref(),
        Some(&b.setup)
    );
    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(snapshot.streams.len(), 2);
    let c = first.command;
    s.acknowledge(
        c.runner_id,
        c.id,
        AckCommand {
            request_id: RequestId::new(),
            ownership_epoch: 1,
        },
    )
    .unwrap();
    s.complete(
        c.runner_id,
        c.id,
        CommandResult {
            request_id: RequestId::new(),
            ownership_epoch: 1,
            result: CommandOutput::Workspace {
                workspace: Workspace {
                    path: "/work".into(),
                    branch: first.stream.branch,
                    base_commit: "a".repeat(40),
                },
            },
        },
    )
    .unwrap();
    let owner = s
        .snapshot()
        .unwrap()
        .streams
        .into_iter()
        .find(|s| s.id == c.stream_id)
        .unwrap();
    let next = s
        .enqueue(
            c.stream_id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: owner.revision,
                kind: CommandKind::RunAgent,
                prompt: Some("test".into()),
            },
        )
        .unwrap();
    assert_eq!(next.workspace_spec.unwrap().setup, Some(b.setup));
}
