use factory_core::*;
use factory_protocol::*;
use factoryd::store::Store;

fn status(s: &Store, id: TicketId, status: TicketState) {
    let t = s
        .snapshot()
        .unwrap()
        .tickets
        .into_iter()
        .find(|t| t.id == id)
        .unwrap();
    s.update_ticket(
        id,
        UpdateTicket {
            request_id: RequestId::new(),
            expected_revision: t.revision,
            title: None,
            description: None,
            acceptance_criteria: None,
            priority: None,
            status: Some(status),
        },
    )
    .unwrap();
}
#[test]
fn concurrent_ticks_respect_dependencies_budget_and_restart_without_duplicate_starts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let s = Store::open(&path).unwrap();
    let p = s
        .create_project(CreateProject {
            runner_mapping: None,
            key: None,
            request_id: RequestId::new(),
            name: "Scheduled".into(),
            repository: "/repo".into(),
            base_branch: "main".into(),
        })
        .unwrap();
    s.update_profile(
        p.id,
        UpdateProfile {
            request_id: RequestId::new(),
            expected_revision: p.revision,
            profile: ExecutionProfile {
                provider_id: "openai".into(),
                model_id: "test".into(),
                ..Default::default()
            },
        },
    )
    .unwrap();
    let tickets = (0..3)
        .map(|n| {
            s.create_ticket(CreateTicket {
                request_id: RequestId::new(),
                project_id: p.id,
                title: format!("Task {n}"),
                description: "implement".into(),
                acceptance_criteria: "tests".into(),
                priority: n,
            })
            .unwrap()
        })
        .collect::<Vec<_>>();
    s.set_dependencies(
        tickets[2].id,
        SetDependencies {
            request_id: RequestId::new(),
            expected_revision: tickets[2].revision,
            depends_on: vec![tickets[0].id],
        },
    )
    .unwrap();
    assert!(s
        .set_dependencies(
            tickets[0].id,
            SetDependencies {
                request_id: RequestId::new(),
                expected_revision: tickets[0].revision,
                depends_on: vec![tickets[2].id]
            }
        )
        .is_err());
    for t in &tickets {
        status(&s, t.id, TicketState::Ready);
    }
    let runners = vec![RunnerId::new(), RunnerId::new()];
    s.set_scheduler(
        p.id,
        SetScheduler {
            request_id: RequestId::new(),
            expected_revision: Revision(0),
            enabled: true,
            runner_ids: runners,
            max_parallel: 2,
            start_budget: 2,
        },
    )
    .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let threads = (0..2)
        .map(|_| {
            let path = path.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                let s = Store::open(&path).unwrap();
                b.wait();
                s.scheduler_tick().unwrap();
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    for t in threads {
        t.join().unwrap();
    }
    let snap = s.snapshot().unwrap();
    assert_eq!(snap.streams.len(), 2);
    assert_eq!(snap.commands.len(), 2);
    assert_ne!(snap.streams[0].runner_id, snap.streams[1].runner_id);
    assert!(!snap
        .streams
        .iter()
        .any(|s| s.ticket_ids.contains(&tickets[2].id)));
    assert_eq!(
        s.scheduler_policy(p.id).unwrap().unwrap().remaining_starts,
        0
    );
    for c in snap.commands {
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
                        path: format!("/work/{}", c.stream_id),
                        branch: c.workspace_spec.unwrap().branch,
                        base_commit: "a".repeat(40),
                    },
                },
            },
        )
        .unwrap();
    }
    drop(s);
    let s = Store::open(&path).unwrap();
    s.scheduler_tick().unwrap();
    s.scheduler_tick().unwrap();
    assert_eq!(
        s.snapshot()
            .unwrap()
            .commands
            .iter()
            .filter(|c| c.kind == CommandKind::RunAgent)
            .count(),
        2
    );
    status(&s, tickets[0].id, TicketState::Done);
    s.scheduler_tick().unwrap();
    assert_eq!(s.snapshot().unwrap().streams.len(), 2);
    assert_eq!(
        s.snapshot()
            .unwrap()
            .tickets
            .iter()
            .find(|t| t.id == tickets[2].id)
            .unwrap()
            .status,
        TicketState::Ready
    );
}
