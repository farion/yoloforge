use factory_core::*;
use factory_protocol::*;
use factoryd::store::{Store, StoreError};

fn project(store: &Store) -> Project {
    store
        .create_project(CreateProject {
            request_id: RequestId::new(),
            name: "knowledge".into(),
            repository: "/repo".into(),
            base_branch: "main".into(),
        })
        .unwrap()
}
fn ticket(store: &Store, p: ProjectId) -> Ticket {
    store
        .create_ticket(CreateTicket {
            request_id: RequestId::new(),
            project_id: p,
            title: "task".into(),
            description: String::new(),
            acceptance_criteria: String::new(),
            priority: 1,
        })
        .unwrap()
}
fn note(project: ProjectId, ticket: Option<TicketId>, title: &str) -> CreateKnowledge {
    CreateKnowledge {
        request_id: RequestId::new(),
        project_id: project,
        ticket_id: ticket,
        title: title.into(),
        content: "Entscheidung mit Grüße und Unicode 🛠".into(),
        source: "operator decision".into(),
        source_command: None,
        priority: 5,
    }
}

#[test]
fn scoped_budgeted_context_retirement_and_deduplication_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let p = project(&store);
    let other = project(&store);
    let t = ticket(&store, p.id);
    let foreign = ticket(&store, other.id);
    let body = note(p.id, None, "global");
    let n = store.create_knowledge(body.clone()).unwrap();
    assert_eq!(store.create_knowledge(body.clone()).unwrap(), n);
    let mut changed = body;
    changed.content = "changed".into();
    assert!(matches!(
        store.create_knowledge(changed),
        Err(StoreError::RequestConflict)
    ));
    let scoped = store
        .create_knowledge(note(p.id, Some(t.id), "ticket only"))
        .unwrap();
    store
        .create_knowledge(note(other.id, None, "foreign secret"))
        .unwrap();
    assert!(store
        .create_knowledge(note(p.id, Some(foreign.id), "invalid"))
        .is_err());
    let mut invalid = note(p.id, None, "invalid source");
    invalid.source_command = Some(CommandId::new());
    assert!(store.create_knowledge(invalid).is_err());
    let global = store.knowledge_context(p.id, None, 8192).unwrap();
    assert_eq!(global.notes, vec![n.clone()]);
    let both = store.knowledge_context(p.id, Some(t.id), 8192).unwrap();
    assert_eq!(both.notes.len(), 2);
    assert!(!both.text.contains("foreign secret"));
    let zero = store.knowledge_context(p.id, Some(t.id), 0).unwrap();
    assert!(zero.notes.is_empty());
    assert_eq!(zero.omitted, 2);
    let bounded = store
        .knowledge_context(p.id, Some(t.id), global.text.len())
        .unwrap();
    assert!(bounded.text.len() <= global.text.len());
    assert_eq!(bounded.notes.len() + bounded.omitted, 2);
    assert!(store
        .knowledge_context(p.id, Some(foreign.id), 8192)
        .is_err());
    assert!(matches!(
        store.retire_knowledge(
            scoped.id,
            RetireKnowledge {
                request_id: RequestId::new(),
                expected_revision: Revision(0)
            }
        ),
        Err(StoreError::RevisionConflict(_))
    ));
    let retire = RetireKnowledge {
        request_id: RequestId::new(),
        expected_revision: Revision(1),
    };
    let retired = store.retire_knowledge(scoped.id, retire.clone()).unwrap();
    assert!(retired.retired);
    assert_eq!(retired.revision, Revision(2));
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.retire_knowledge(scoped.id, retire).unwrap(), retired);
    assert_eq!(
        store
            .knowledge_context(p.id, Some(t.id), 8192)
            .unwrap()
            .notes,
        vec![n]
    );
    assert_eq!(store.knowledge(p.id).unwrap().len(), 2);
}

#[test]
fn queued_agent_prompt_keeps_selected_knowledge_after_retirement_and_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let p = project(&store);
    let runner = RunnerId::new();
    let t = ticket(&store, p.id);
    store
        .update_profile(
            p.id,
            UpdateProfile {
                request_id: RequestId::new(),
                expected_revision: p.revision,
                profile: ExecutionProfile {
                    provider_id: "openai".into(),
                    model_id: "gpt-6-astra".into(),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    let stream = store
        .create_stream(CreateStream {
            request_id: RequestId::new(),
            project_id: p.id,
            runner_id: runner,
            goal: "task".into(),
            ticket_ids: vec![t.id],
        })
        .unwrap();
    let prep = store
        .enqueue(
            stream.id,
            EnqueueCommand {
                request_id: RequestId::new(),
                expected_revision: stream.revision,
                kind: CommandKind::PrepareWorkspace,
                prompt: None,
            },
        )
        .unwrap();
    store
        .acknowledge(
            runner,
            prep.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
    store
        .complete(
            runner,
            prep.id,
            CommandResult {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                result: CommandOutput::Workspace {
                    workspace: Workspace {
                        path: "/prepared".into(),
                        branch: stream.branch,
                        base_commit: "a".repeat(40),
                    },
                },
            },
        )
        .unwrap();
    let mut body = note(p.id, Some(t.id), "selected decision");
    body.source_command = Some(prep.id);
    let n = store.create_knowledge(body).unwrap();
    let input = EnqueueCommand {
        request_id: RequestId::new(),
        expected_revision: store.snapshot().unwrap().streams[0].revision,
        kind: CommandKind::RunAgent,
        prompt: Some("Implement task".into()),
    };
    let command = store.enqueue(stream.id, input.clone()).unwrap();
    assert!(command
        .agent_spec
        .as_ref()
        .unwrap()
        .prompt
        .contains(&n.id.to_string()));
    store
        .retire_knowledge(
            n.id,
            RetireKnowledge {
                request_id: RequestId::new(),
                expected_revision: n.revision,
            },
        )
        .unwrap();
    assert!(store
        .knowledge_context(p.id, Some(t.id), 8192)
        .unwrap()
        .notes
        .is_empty());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.enqueue(stream.id, input).unwrap(), command);
}
