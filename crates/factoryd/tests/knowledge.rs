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
        branch: None,
        supersedes: None,
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
fn queued_agent_prompt_keeps_branch_selected_knowledge_after_supersession_and_replay() {
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
                        branch: stream.branch.clone(),
                        base_commit: "a".repeat(40),
                    },
                },
            },
        )
        .unwrap();
    let mut body = note(p.id, Some(t.id), "selected decision");
    body.source_command = Some(prep.id);
    body.branch = Some(stream.branch.clone());
    let mut wrong = body.clone();
    wrong.request_id = RequestId::new();
    wrong.branch = Some("other/branch".into());
    assert!(store.create_knowledge(wrong).is_err());
    let mut unrelated = note(p.id, Some(t.id), "foreign branch decision");
    unrelated.branch = Some("other/branch".into());
    let unrelated = store.create_knowledge(unrelated).unwrap();
    let n = store.create_knowledge(body).unwrap();
    let input = EnqueueCommand {
        request_id: RequestId::new(),
        expected_revision: store.snapshot().unwrap().streams[0].revision,
        kind: CommandKind::RunAgent,
        prompt: Some("Implement task".into()),
    };
    let command = store.enqueue(stream.id, input.clone()).unwrap();
    assert!(!command
        .agent_spec
        .as_ref()
        .unwrap()
        .prompt
        .contains(&unrelated.id.to_string()));
    assert!(command
        .agent_spec
        .as_ref()
        .unwrap()
        .prompt
        .contains(&n.id.to_string()));
    let mut replacement = note(p.id, Some(t.id), "corrected decision");
    replacement.branch = Some(stream.branch.clone());
    replacement.source_command = Some(prep.id);
    replacement.supersedes = Some(SupersedeKnowledge {
        id: n.id,
        expected_revision: n.revision,
    });
    let replacement = store.create_knowledge(replacement).unwrap();
    assert_eq!(
        store
            .knowledge_context_for_branch(p.id, Some(t.id), Some(&stream.branch), 8192)
            .unwrap()
            .notes,
        vec![replacement]
    );
    assert!(store
        .knowledge_context(p.id, Some(t.id), 8192)
        .unwrap()
        .notes
        .is_empty());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.enqueue(stream.id, input).unwrap(), command);
}

#[test]
fn branch_filtering_precedes_candidate_limit_and_preserves_utf8_budget() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let p = project(&store);
    let global = store.create_knowledge(note(p.id, None, "global")).unwrap();
    let mut local = note(p.id, None, "branch-specific");
    local.branch = Some("feature/grüße".into());
    let local = store.create_knowledge(local).unwrap();
    for i in 0..257 {
        let mut other = note(p.id, None, &format!("other {i}"));
        other.branch = Some("other".into());
        other.priority = 9;
        store.create_knowledge(other).unwrap();
    }
    let context = store
        .knowledge_context_for_branch(p.id, None, Some("feature/grüße"), 8192)
        .unwrap();
    assert_eq!(context.notes.len(), 2);
    assert_eq!(context.omitted, 0);
    assert!(context.notes.contains(&local));
    assert!(context.notes.contains(&global));
    assert_eq!(store.knowledge_note(global.id).unwrap(), global);
    assert!(matches!(
        store.knowledge_note(KnowledgeId::new()),
        Err(StoreError::NotFound)
    ));
    assert!(context.text.contains("branch: feature/grüße"));
    assert_eq!(
        store.knowledge_context(p.id, None, 8192).unwrap().notes,
        vec![global]
    );
    let short = store
        .knowledge_context_for_branch(p.id, None, Some("feature/grüße"), context.text.len() - 1)
        .unwrap();
    assert!(short.text.len() < context.text.len());
    assert_eq!(short.notes.len() + short.omitted, 2);
    for invalid in [
        "",
        "bad branch",
        "../bad",
        "refs//bad",
        "HEAD",
        "branch.lock",
        "x@{y",
        "x\n",
    ] {
        let mut body = note(p.id, None, "invalid");
        body.branch = Some(invalid.into());
        assert!(store.create_knowledge(body).is_err());
        assert!(store
            .knowledge_context_for_branch(p.id, None, Some(invalid), 8192)
            .is_err());
    }
}

#[test]
fn concurrent_supersession_has_one_winner_and_durable_bidirectional_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let p = project(&store);
    let original = store
        .create_knowledge(note(p.id, None, "original"))
        .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|index| {
            let path = path.clone();
            let barrier = barrier.clone();
            let mut body = note(p.id, None, &format!("replacement {index}"));
            body.supersedes = Some(SupersedeKnowledge {
                id: original.id,
                expected_revision: original.revision,
            });
            std::thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                barrier.wait();
                let result = store.create_knowledge(body.clone());
                (body, result)
            })
        })
        .collect::<Vec<_>>();
    let outcomes = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.iter().filter(|(_, r)| r.is_ok()).count(), 1);
    assert!(outcomes
        .iter()
        .any(|(_, r)| matches!(r, Err(StoreError::RevisionConflict(Revision(2))))));
    let (request, winner) = outcomes.into_iter().find(|(_, r)| r.is_ok()).unwrap();
    let winner = winner.unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.create_knowledge(request).unwrap(), winner);
    let notes = store.knowledge(p.id).unwrap();
    assert_eq!(notes.len(), 2);
    let old = notes.iter().find(|n| n.id == original.id).unwrap();
    assert!(old.retired);
    assert_eq!(old.superseded_by, Some(winner.id));
    assert_eq!(winner.supersedes, Some(old.id));
    assert_eq!(old.content, original.content);
    assert_eq!(
        store.knowledge_context(p.id, None, 8192).unwrap().notes,
        vec![winner.clone()]
    );
    let mut cross_scope = note(p.id, None, "scope change");
    cross_scope.branch = Some("other".into());
    cross_scope.supersedes = Some(SupersedeKnowledge {
        id: winner.id,
        expected_revision: winner.revision,
    });
    assert!(store.create_knowledge(cross_scope).is_err());
    assert_eq!(store.knowledge(p.id).unwrap().len(), 2);
}

#[test]
fn failed_supersession_rolls_back_both_notes_and_request_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let p = project(&store);
    let original = store
        .create_knowledge(note(p.id, None, "original"))
        .unwrap();
    let mut replacement = note(p.id, None, "replacement");
    replacement.supersedes = Some(SupersedeKnowledge {
        id: original.id,
        expected_revision: original.revision,
    });
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_supersede BEFORE INSERT ON events WHEN NEW.kind='knowledge.superseded' BEGIN SELECT RAISE(ABORT,'test rollback'); END;").unwrap();
    assert!(store.create_knowledge(replacement.clone()).is_err());
    assert_eq!(store.knowledge(p.id).unwrap(), vec![original.clone()]);
    db.execute_batch("DROP TRIGGER fail_supersede").unwrap();
    let winner = store.create_knowledge(replacement.clone()).unwrap();
    assert_eq!(store.create_knowledge(replacement).unwrap(), winner);
    assert_eq!(winner.supersedes, Some(original.id));
}

#[test]
fn legacy_curated_notes_keep_global_scope_and_request_replay_after_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let p = project(&store);
    let body = note(p.id, None, "legacy");
    let saved = store.create_knowledge(body.clone()).unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE scheduler_runs; DROP TABLE scheduler_policies; DROP TABLE reviews; DROP TABLE checkpoint_bundles; DROP TABLE checkpoints; DROP TABLE runtime_bindings; DROP TABLE runtime_terminations; DROP INDEX knowledge_branch_scope; PRAGMA user_version=11;")
        .unwrap();
    drop(db);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.create_knowledge(body).unwrap(), saved);
    assert_eq!(
        store
            .knowledge_context_for_branch(p.id, None, Some("any/branch"), 8192)
            .unwrap()
            .notes,
        vec![saved]
    );
}
