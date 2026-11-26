use factory_core::*;
use factory_protocol::*;
use factoryd::store::Store;

async fn post_prompt(store: &Store, path: &str, body: &impl serde::Serialize) -> Ticket {
    const TOKEN: &str = "test-only-credential-0123456789abcdef";
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let router = factoryd::router(TOKEN, store.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = factory_protocol::client::FactoryClient::new(&url, TOKEN).unwrap();
    let result = client
        .management_request("POST", path, &serde_json::to_value(body).unwrap())
        .await;
    server.abort();
    serde_json::from_value(result.unwrap()).unwrap()
}

fn configure_models(s: &Store) {
    for p in s.snapshot().unwrap().projects {
        if p.profile.provider_id.is_empty() {
            s.update_profile(
                p.id,
                UpdateProfile {
                    request_id: RequestId::new(),
                    expected_revision: p.revision,
                    profile: ExecutionProfile {
                        provider_id: "openai".into(),
                        model_id: "test-configured".into(),
                        ..p.profile
                    },
                },
            )
            .unwrap();
        }
    }
}

#[tokio::test]
async fn long_http_prompt_reaches_orchestrator_and_worker_without_truncation() {
    let store = Store::open(std::path::Path::new(":memory:")).unwrap();
    let project = store.create_project(CreateProject {
        logo: None, runner_mapping: None, key: None, request_id: RequestId::new(),
        name: "Long prompt".into(), repository: "factory:managed".into(), base_branch: "main".into(),
    }).unwrap();
    configure_models(&store);
    let runner = RunnerId::new();
    let prompt = format!("Implement changes\n{}Do not push.", "Details: ä, ö, ü.\n".repeat(3_000));
    let ticket = post_prompt(&store, "/v1/tasks/submit-with-model", &models::SubmitTaskWithModel {
        submission: SubmitTask {
            task: SaveTask {
                request_id: RequestId::new(), id: None, expected_revision: Revision(0),
                project_id: project.id, requested_session: None, prompt: prompt.clone(), images: vec![],
            },
            edit_token: None,
            draft: false,
            files: vec![],
        },
        implementer: None,
        local_mcps: None,
    }).await;
    assert_eq!(ticket.prompt.as_deref(), Some(prompt.as_str()));
    store.autostart_runner(runner).unwrap();
    store.scheduler_tick().unwrap();
    prepare(&store, &store.pending(runner).unwrap()[0]);
    store.scheduler_tick().unwrap();
    let routing = store.pending(runner).unwrap().pop().unwrap();
    let spec = routing.agent_spec.as_ref().unwrap();
    assert_eq!(spec.role, "orchestrator");
    assert!(spec.prompt.contains(&format!("\nTask: {}\n", serde_json::to_string(&prompt).unwrap())));
    decide(&store, &routing, r#"{"action":"new","reason":"New workspace"}"#);
    store.scheduler_tick().unwrap();
    prepare(&store, &store.pending(runner).unwrap()[0]);
    store.scheduler_tick().unwrap();
    let worker = store.pending(runner).unwrap().pop().unwrap();
    let spec = worker.agent_spec.unwrap();
    assert!(!spec.read_only);
    assert!(spec.prompt.starts_with(&prompt));
}

#[test]
fn long_prompt_reaches_orchestrator_and_worker_without_truncation() {
    let store = Store::open(std::path::Path::new(":memory:")).unwrap();
    let project = store.create_project(CreateProject {
        logo: None, runner_mapping: None, key: None, request_id: RequestId::new(),
        name: "Long prompt".into(), repository: "factory:managed".into(), base_branch: "main".into(),
    }).unwrap();
    configure_models(&store);
    let runner = RunnerId::new();
    let prompt = format!("Implement changes\n{}Do not push.", "Details: ä, ö, ü.\n".repeat(3_000));
    let ticket = store.save_task(SaveTask {
        request_id: RequestId::new(), id: None, expected_revision: Revision(0),
        project_id: project.id, requested_session: None, prompt: prompt.clone(), images: vec![],
    }).unwrap();
    assert_eq!(ticket.prompt.as_deref(), Some(prompt.as_str()));
    store.autostart_runner(runner).unwrap();
    store.scheduler_tick().unwrap();
    prepare(&store, &store.pending(runner).unwrap()[0]);
    store.scheduler_tick().unwrap();
    let routing = store.pending(runner).unwrap().pop().unwrap();
    let spec = routing.agent_spec.as_ref().unwrap();
    assert_eq!(spec.role, "orchestrator");
    assert!(spec.prompt.contains(&format!("\nTask: {}\n", serde_json::to_string(&prompt).unwrap())));
    decide(&store, &routing, r#"{"action":"new","reason":"New workspace"}"#);
    store.scheduler_tick().unwrap();
    prepare(&store, &store.pending(runner).unwrap()[0]);
    store.scheduler_tick().unwrap();
    let worker = store.pending(runner).unwrap().pop().unwrap();
    let spec = worker.agent_spec.unwrap();
    assert!(!spec.read_only);
    assert!(spec.prompt.starts_with(&prompt));
}

#[test]
fn skills_reach_only_the_implementer_and_are_pinned_before_routing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let project = store.create_project(CreateProject {
        logo: None,
        runner_mapping: None,
        key: None,
        request_id: RequestId::new(),
        name: "Skills".into(),
        repository: "factory:managed".into(),
        base_branch: "main".into(),
    }).unwrap();
    configure_models(&store);
    let package = store.import_factory_skill(factory_protocol::skills::ImportFactorySkill {
        request_id:RequestId::new(),id:"checklist".into(), name:"Checklist".into(),
        description:"Review checklist".into(), current_version:0,
        files:[("SKILL.md".into(),"Review carefully".into())].into(),
    }).unwrap();
    let change = |revision, id: &str| store.set_project_skills(project.id, SetProjectSkills {
        request_id: RequestId::new(),
        expected_revision: Revision(revision),
        repo_skills: vec![id.into()],
        factory_skills: if revision == 0 {vec![package.reference.clone()]} else {vec![]},
    }).unwrap();
    change(0, "review");
    let task = |requested_session| SaveTask {
        request_id: RequestId::new(), id: None, expected_revision: Revision(0),
        project_id: project.id, prompt: "Work".into(), images: vec![], requested_session,
    };
    let ticket = store.save_task(task(None)).unwrap();
    change(1, "docs");
    let runner = RunnerId::new();
    store.autostart_runner(runner).unwrap();
    store.scheduler_tick().unwrap();
    prepare(&store, &store.pending(runner).unwrap()[0]);
    store.scheduler_tick().unwrap();
    let routing = store.pending(runner).unwrap().pop().unwrap();
    assert!(routing.agent_spec.as_ref().unwrap().read_only);
    assert!(routing.agent_spec.as_ref().unwrap().repo_skills.is_none());
    assert!(routing.agent_spec.as_ref().unwrap().factory_skills.is_empty());
    decide(&store, &routing, r#"{"action":"new","reason":"New workspace"}"#);
    store.scheduler_tick().unwrap();
    let preparation = store.pending(runner).unwrap().pop().unwrap();
    prepare(&store, &preparation);
    store.scheduler_tick().unwrap();
    let implementer = store.pending(runner).unwrap().pop().unwrap();
    assert!(!implementer.agent_spec.as_ref().unwrap().read_only);
    assert_eq!(implementer.agent_spec.as_ref().unwrap().repo_skills.as_deref(), Some(&["review".into()][..]));
    assert_eq!(implementer.agent_spec.as_ref().unwrap().factory_skills, [package.reference.clone()]);
    assert!(store.require_repo_skill_support(runner, implementer.id, implementer.ownership_epoch, false).is_err());
    store.require_repo_skill_support(runner, implementer.id, implementer.ownership_epoch, true).unwrap();
    assert!(store.snapshot().unwrap().tickets.iter().any(|t| {
        t.id == ticket.id && t.execution_command == Some(implementer.id)
    }));
    assert!(matches!(store.save_task(task(Some(implementer.id))),
        Err(factoryd::store::StoreError::Invalid(message)) if message.contains("Skill-Freigabe")));
    assert!(store.factory_skill_for_command(runner,implementer.id,package.reference.clone()).is_err());
    store.acknowledge(runner,implementer.id,AckCommand {request_id:RequestId::new(),
        ownership_epoch:implementer.ownership_epoch}).unwrap();
    assert!(store.factory_skill_for_command(RunnerId::new(),implementer.id,package.reference.clone()).is_err());
    let bundle = store.factory_skill_for_command(runner,implementer.id,package.reference).unwrap();
    assert_eq!(bundle.files["SKILL.md"],"Review carefully");
}

#[test]
fn orchestrator_retries_failed_preparation_after_cooldown_and_restart() {
    check_preparation_retry("OpenCode unavailable", false);
}

#[test]
fn inaccessible_project_path_waits_on_its_assigned_runner_and_recovers() {
    check_preparation_retry(
        "Cannot create project parent folder on the runner: Permission denied (os error 13); path: /opt/repos",
        true,
    );
}

fn check_preparation_retry(error: &str, access_denied: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let s = Store::open(&path).unwrap();
    let p = s.create_project(CreateProject {
        logo: None,
        runner_mapping: None,
        key: None,
        request_id: RequestId::new(),
        name: "Recover".into(),
        repository: "factory:managed".into(),
        base_branch: "main".into(),
    }).unwrap();
    configure_models(&s);
    let ticket = s.save_task(SaveTask {
        request_id: RequestId::new(),
        id: None,
        expected_revision: Revision(0),
        project_id: p.id,
        prompt: "Do work".into(),
        images: vec![],
        requested_session: None,
    }).unwrap();
    let runner = RunnerId::new();
    s.autostart_runner(runner).unwrap();
    s.scheduler_tick().unwrap();
    let first = s.pending(runner).unwrap().pop().unwrap();
    assert_eq!(first.kind, CommandKind::PrepareWorkspace);
    complete(&s, &first, CommandOutput::WorkspaceError {
        workspace_error: error.into(),
    });
    s.scheduler_tick().unwrap();
    assert!(s.pending(runner).unwrap().is_empty());
    assert_eq!(s.notifications("test", None).unwrap().entries[0].command_id, Some(first.id));
    let history = s.task_history(ticket.id, None).unwrap();
    if access_denied {
        assert_eq!(s.task_payload(ticket.id).unwrap().routing_note,
            "Orchestrator wartet: Runner hat keinen Zugriff auf den Projektpfad");
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute("UPDATE events SET created_unix_ms=?1 WHERE aggregate_id=?2 AND kind='command.updated'",
            rusqlite::params![now - 120_000, first.id.to_string()]).unwrap();
        drop(db);
        s.scheduler_tick().unwrap();
        assert!(s.pending(runner).unwrap().is_empty(), "Do not retry a permission error every minute");
    } else {
        assert!(history.entries.iter().any(|entry| {
            entry.level == TaskHistoryLevel::Error
                && entry.message.starts_with("Orchestrator-Vorbereitung fehlgeschlagen")
                && entry.created_unix_ms > 0
        }));
    }
    assert!(history.entries.iter().any(|entry| entry.message == "Aufgabe eingereiht"));
    assert!(history.entries.windows(2).all(|pair| pair[0].sequence > pair[1].sequence));
    drop(s);

    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE events SET created_unix_ms=0 WHERE aggregate_id=?1 AND kind='command.updated'",
        [first.id.to_string()]).unwrap();
    drop(db);
    let s = Store::open(&path).unwrap();
    s.scheduler_tick().unwrap();
    s.scheduler_tick().unwrap();
    let pending = s.pending(runner).unwrap();
    assert_eq!(pending.len(), 1);
    let retry = &pending[0];
    assert_eq!(retry.kind, CommandKind::PrepareWorkspace);
    assert_eq!(retry.stream_id, first.stream_id);
    assert_ne!(retry.id, first.id);
    prepare(&s, retry);
    s.scheduler_tick().unwrap();
    let decision = s.pending(runner).unwrap().pop().unwrap();
    decide(&s, &decision, r#"{"action":"new","reason":"Recovered"}"#);
    s.scheduler_tick().unwrap();
    assert!(s.snapshot().unwrap().streams.iter().any(|w| w.ticket_ids == [ticket.id]));
    let history = s.task_history(ticket.id, None).unwrap();
    assert!(history.entries.iter().any(|entry| entry.message == "Arbeitsbereich wird vorbereitet"));
}

#[test]
fn automatic_setup_preserves_user_model_and_missing_model_blocks_agents() {
    for configured in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let s = Store::open(&path).unwrap();
        let p = s
            .create_project(CreateProject {
                logo: None,
                runner_mapping: None,
                key: None,
                request_id: RequestId::new(),
                name: "Pinned".into(),
                repository: "factory:managed".into(),
                base_branch: "main".into(),
            })
            .unwrap();
        if configured {
            configure_models(&s);
        }
        let before = s.snapshot().unwrap().projects[0].profile.clone();
        s.create_ticket(CreateTicket {
            request_id: RequestId::new(),
            project_id: p.id,
            title: "Work".into(),
            description: "Work".into(),
            acceptance_criteria: String::new(),
            priority: 1,
        })
        .unwrap();
        let runner = RunnerId::new();
        s.autostart_runner(runner).unwrap();
        assert_eq!(s.snapshot().unwrap().projects[0].profile, before);
        s.scheduler_tick().unwrap();
        for command in s.pending(runner).unwrap() {
            prepare(&s, &command);
        }
        s.scheduler_tick().unwrap();
        let snapshot = s.snapshot().unwrap();
        let agents: Vec<_> = snapshot
            .commands
            .iter()
            .filter(|c| c.kind == CommandKind::RunAgent)
            .collect();
        if configured {
            assert_eq!(agents.len(), 1);
            assert_eq!(agents[0].agent_spec.as_ref().unwrap().profile, before);
        } else {
            assert!(agents.is_empty());
        }
        drop(s);
        assert_eq!(
            Store::open(&path).unwrap().snapshot().unwrap().projects[0].profile,
            before
        );
    }
}

#[test]
fn completed_task_memory_is_once_only_provenanced_and_branch_scoped() {
    for output in [
        r#"{"memory":"The task reported a calculator implementation; test success is an agent claim."}"#,
        r#"{"memory":"Racing calculator implementation observation."}"#,
        r#"{"memory":""}"#,
        r#"{"memory":"Replace an unoffered note","supersedes":"00000000-0000-4000-8000-000000000001"}"#,
        "invalid",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let s = Store::open(&path).unwrap();
        let p = s
            .create_project(CreateProject {
                logo: None,
                runner_mapping: None,
                key: None,
                request_id: RequestId::new(),
                name: "Memory".into(),
                repository: "factory:managed".into(),
                base_branch: "main".into(),
            })
            .unwrap();
        s.create_ticket(CreateTicket {
            request_id: RequestId::new(),
            project_id: p.id,
            title: "Calculator".into(),
            description: "Build calculator".into(),
            acceptance_criteria: String::new(),
            priority: 1,
        })
        .unwrap();
        let runner = RunnerId::new();
        configure_models(&s);
        s.autostart_runner(runner).unwrap();
        s.scheduler_tick().unwrap();
        prepare(&s, &s.pending(runner).unwrap()[0]);
        s.scheduler_tick().unwrap();
        let work = s.pending(runner).unwrap()[0].clone();
        complete(
            &s,
            &work,
            CommandOutput::Agent {
                agent: AgentProgress {
                    session_id: work.session_id(),
                    message_id: agent_message_id(work.id),
                    phase: AgentPhase::Succeeded,
                    summary: "Implemented calculator; tests pass".into(),
                },
            },
        );
        s.scheduler_tick().unwrap();
        prepare(&s, &s.pending(runner).unwrap()[0]);
        s.scheduler_tick().unwrap();
        let review = s.pending(runner).unwrap().pop().unwrap();
        assert_eq!(review.agent_spec.as_ref().unwrap().role, "reviewer");
        agent_reply(&s, &review, r#"{"action":"approve"}"#);
        s.scheduler_tick().unwrap();
        assert!(s.pending(runner).unwrap().is_empty(), "Memory must wait for final archive");
        s.report_archive(
            runner,
            work.id,
            ReportArchive {
                ownership_epoch: 1,
                batch: ArchiveBatch {
                    session_id: work.session_id(),
                    revision: 1,
                    captured_unix_ms: 1,
                    final_scan_complete: true,
                    messages: vec![],
                },
            },
        )
        .unwrap();
        s.scheduler_tick().unwrap();
        let eval = s.pending(runner).unwrap()[0].clone();
        assert_eq!(
            eval.agent_spec.as_ref().unwrap().role,
            "memorizer"
        );
        assert!(eval.agent_spec.as_ref().unwrap().read_only);
        let prompt = &eval.agent_spec.as_ref().unwrap().prompt;
        assert!(prompt.contains("You are the YoloForge Memorizer."));
        assert!(!prompt.contains("Supervisor evaluation only"));
        complete(
            &s,
            &eval,
            CommandOutput::Agent {
                agent: AgentProgress {
                    session_id: eval.session_id(),
                    message_id: agent_message_id(eval.id),
                    phase: AgentPhase::Succeeded,
                    summary: output.into(),
                },
            },
        );
        drop(s);
        let s = Store::open(&path).unwrap();
        for _ in 0..3 {
            s.scheduler_tick().unwrap();
        }
        let notes = s.knowledge(p.id).unwrap();
        if output.contains("calculator implementation") {
            assert_eq!(notes.len(), 1);
            let note = &notes[0];
            assert_eq!(note.source_command, Some(work.id));
            assert!(note.source.contains(&eval.id.to_string()));
            let branch = s
                .snapshot()
                .unwrap()
                .streams
                .iter()
                .find(|stream| stream.id == work.stream_id)
                .unwrap()
                .branch
                .clone();
            assert_eq!(note.branch.as_deref(), Some(branch.as_str()));
            assert!(s
                .knowledge_context_for_branch(p.id, None, Some(&branch), 4096)
                .unwrap()
                .text
                .contains("calculator implementation"));
            assert!(!s
                .knowledge_context_for_branch(p.id, None, Some("main"), 4096)
                .unwrap()
                .text
                .contains("calculator implementation"));
        } else {
            assert!(notes.is_empty());
        }
        assert_eq!(s.snapshot().unwrap().commands.len(), 5);
        if output.contains("calculator implementation") {
            let old = notes[0].clone();
            s.save_task(SaveTask {
                request_id: RequestId::new(),
                id: None,
                expected_revision: Revision(0),
                project_id: p.id,
                prompt: "Correct the calculator tests".into(),
                images: vec![],
                requested_session: Some(work.id),
            })
            .unwrap();
            s.scheduler_tick().unwrap();
            let follow = s
                .pending(runner)
                .unwrap()
                .into_iter()
                .find(|c| {
                    c.kind == CommandKind::RunAgent && !c.agent_spec.as_ref().unwrap().read_only
                })
                .unwrap();
            complete(
                &s,
                &follow,
                CommandOutput::Agent {
                    agent: AgentProgress {
                        session_id: follow.session_id(),
                        message_id: agent_message_id(follow.id),
                        phase: AgentPhase::Succeeded,
                        summary: "Earlier tests were incomplete; now added negative-number cases"
                            .into(),
                    },
                },
            );
            s.report_archive(
                runner,
                follow.id,
                ReportArchive {
                    ownership_epoch: 1,
                    batch: ArchiveBatch {
                        session_id: follow.session_id(),
                        revision: 1,
                        captured_unix_ms: 2,
                        final_scan_complete: true,
                        messages: vec![],
                    },
                },
            )
            .unwrap();
            s.scheduler_tick().unwrap();
            let review = s.pending(runner).unwrap().pop().unwrap();
            assert_eq!(review.agent_spec.as_ref().unwrap().role, "reviewer");
            agent_reply(&s, &review, r#"{"action":"approve"}"#);
            s.scheduler_tick().unwrap();
            let evaluation = s
                .pending(runner)
                .unwrap()
                .into_iter()
                .find(|c| {
                    c.agent_spec
                        .as_ref()
                        .is_some_and(|a| a.role == "memorizer")
                })
                .unwrap();
            assert!(evaluation
                .agent_spec
                .as_ref()
                .unwrap()
                .prompt
                .contains(&old.id.to_string()));
            let racing = output.contains("Racing");
            if racing {
                s.retire_knowledge(
                    old.id,
                    RetireKnowledge {
                        request_id: RequestId::new(),
                        expected_revision: old.revision,
                    },
                )
                .unwrap();
            }
            complete(&s,&evaluation,CommandOutput::Agent{agent:AgentProgress{session_id:evaluation.session_id(),message_id:agent_message_id(evaluation.id),phase:AgentPhase::Succeeded,summary:serde_json::json!({"memory":"Correction: earlier tests incomplete; negative-number tests now reported; calculator implementation retained.","supersedes":old.id}).to_string()}});
            drop(s);
            let s = Store::open(&path).unwrap();
            s.scheduler_tick().unwrap();
            s.scheduler_tick().unwrap();
            let previous = s.knowledge_note(old.id).unwrap();
            assert!(previous.retired);
            if racing {
                assert!(previous.superseded_by.is_none());
            } else {
                let replacement = s.knowledge_note(previous.superseded_by.unwrap()).unwrap();
                assert_eq!(replacement.supersedes, Some(old.id));
                assert_eq!(replacement.source_command, Some(follow.id));
                assert_eq!(replacement.branch, old.branch);
                assert!(replacement.content.contains("Correction"));
            }
        }
    }
}

#[test]
fn forms_wait_for_humans_and_permission_orchestration_remains_once_only() {
    for (human, reply, expected) in [
        (
            false,
            r#"{"action":"answer","answer":{"decision":"once"},"reason":"Matches the task"}"#,
            Some("once"),
        ),
        (
            true,
            r#"{"action":"answer","answer":{"decision":"once"},"reason":"Matches the task"}"#,
            Some("reject"),
        ),
        (
            false,
            r#"{"action":"escalate","reason":"User preference is missing"}"#,
            None,
        ),
        (
            false,
            r#"{"action":"answer","answer":{"decision":"always"},"reason":"Unsupported choice"}"#,
            None,
        ),
        (false, "interview", None),
        (false, "ordinary form", None),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let s = Store::open(&path).unwrap();
        let p = s
            .create_project(CreateProject {
                logo: None,
                runner_mapping: None,
                key: None,
                request_id: RequestId::new(),
                name: "Questions".into(),
                repository: "factory:managed".into(),
                base_branch: "main".into(),
            })
            .unwrap();
        s.create_ticket(CreateTicket {
            request_id: RequestId::new(),
            project_id: p.id,
            title: "Build blue UI".into(),
            description: if reply == "interview" {
                "Use blue. Grill me"
            } else {
                "Use blue"
            }
            .into(),
            acceptance_criteria: String::new(),
            priority: 1,
        })
        .unwrap();
        let runner = RunnerId::new();
        configure_models(&s);
        s.autostart_runner(runner).unwrap();
        s.scheduler_tick().unwrap();
        prepare(&s, &s.pending(runner).unwrap()[0]);
        s.scheduler_tick().unwrap();
        let worker = s.pending(runner).unwrap()[0].clone();
        s.acknowledge(
            runner,
            worker.id,
            AckCommand {
                request_id: RequestId::new(),
                ownership_epoch: 1,
            },
        )
        .unwrap();
        let form = matches!(reply, "interview" | "ordinary form");
        let external = ExternalQuestion {
            external_id: "request_test".into(),
            kind: if form { QuestionKind::Form } else { QuestionKind::Permission },
            payload: if form {
                serde_json::json!({"id":"request_test","sessionID":worker.session_id(),"title":"Which color?","fields":[{"key":"color","type":"string","required":true}]})
            } else {
                serde_json::json!({"id":"request_test","sessionID":worker.session_id(),"action":"read","resources":["resource"]})
            },
        };
        let question = s.sync_questions(runner, worker.id, QuestionSnapshot {
            ownership_epoch: 1,
            questions: vec![external.clone()],
        }).unwrap()[0].clone();
        s.scheduler_tick().unwrap();
        if form {
            drop(s);
            let s = Store::open(&path).unwrap();
            for _ in 0..3 {
                s.scheduler_tick().unwrap();
            }
            let snapshot = s.snapshot().unwrap();
            assert_eq!(snapshot.questions[0].state, QuestionState::Pending);
            assert!(snapshot.questions[0].answer.is_none());
            assert_eq!(snapshot.questions[0], question);
            assert!(!snapshot.commands.iter().any(|c| c
                .agent_spec
                .as_ref()
                .is_some_and(|a| a.role == "orchestrator-question")));
            let queued = s.answer_question(question.id, AnswerQuestion {
                request_id: RequestId::new(),
                legacy_revision: None,
                answer: serde_json::json!({"color":"red"}),
            }).unwrap();
            let answered = s.answer_delivery(runner, question.id, AnswerDelivery {
                request_id: RequestId::new(),
                ownership_epoch: 1,
                answer_request: queued.answer_request.unwrap(),
                state: QuestionState::Answered,
            }).unwrap();
            let mut repeated_external = external;
            repeated_external.payload["title"] = serde_json::json!("Changed text under the same ID");
            let repeated = s.sync_questions(runner, worker.id, QuestionSnapshot {
                ownership_epoch: 1,
                questions: vec![repeated_external],
            }).unwrap();
            assert_eq!(repeated, vec![answered]);
            continue;
        }
        let prep = s
            .pending(runner)
            .unwrap()
            .into_iter()
            .find(|c| c.kind == CommandKind::PrepareWorkspace)
            .unwrap();
        prepare(&s, &prep);
        s.scheduler_tick().unwrap();
        let eval = s
            .pending(runner)
            .unwrap()
            .into_iter()
            .find(|c| {
                c.agent_spec
                    .as_ref()
                    .is_some_and(|a| a.role == "orchestrator-question")
            })
            .unwrap();
        assert!(eval.agent_spec.as_ref().unwrap().read_only);
        if human {
            s.answer_question(
                question.id,
                AnswerQuestion {
                    request_id: RequestId::new(),
                    legacy_revision: None,
                    answer: serde_json::json!({"decision":"reject"}),
                },
            )
            .unwrap();
        }
        complete(
            &s,
            &eval,
            CommandOutput::Agent {
                agent: AgentProgress {
                    session_id: eval.session_id(),
                    message_id: agent_message_id(eval.id),
                    phase: AgentPhase::Succeeded,
                    summary: reply.into(),
                },
            },
        );
        drop(s);
        let s = Store::open(&path).unwrap();
        s.scheduler_tick().unwrap();
        let q = s.snapshot().unwrap().questions[0].clone();
        assert_eq!(
            q.answer.as_ref().and_then(|v| v["decision"].as_str()),
            expected
        );
        assert_eq!(
            q.state,
            if expected.is_some() {
                QuestionState::AnswerQueued
            } else {
                QuestionState::Pending
            }
        );
        assert!(!q.orchestration_note.is_empty());
        let receipt = q.answer_request;
        for _ in 0..3 {
            s.scheduler_tick().unwrap();
        }
        let snap = s.snapshot().unwrap();
        assert_eq!(snap.questions[0].answer_request, receipt);
        assert_eq!(snap.commands.len(), 4);
    }
}

fn complete(s: &Store, c: &RunnerCommand, result: CommandOutput) {
    s.acknowledge(
        c.runner_id,
        c.id,
        AckCommand {
            request_id: RequestId::new(),
            ownership_epoch: c.ownership_epoch,
        },
    )
    .unwrap();
    s.complete(
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
fn prepare(s: &Store, c: &RunnerCommand) {
    complete(
        s,
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
fn agent_reply(s: &Store, c: &RunnerCommand, text: &str) {
    complete(s, c, CommandOutput::Agent { agent: AgentProgress {
        session_id: c.session_id(), message_id: agent_message_id(c.id),
        phase: AgentPhase::Succeeded, summary: text.into(),
    }});
}

fn final_archive(s: &Store, c: &RunnerCommand) {
    s.report_archive(c.runner_id, c.id, ReportArchive {
        ownership_epoch: c.ownership_epoch,
        batch: ArchiveBatch { session_id: c.session_id(), revision: 1,
            captured_unix_ms: 1, final_scan_complete: true, messages: vec![] },
    }).unwrap();
}

#[test]
fn deleting_an_intent_with_an_open_question_hides_it_and_persists_the_stop_request() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("db");
    let s = Store::open(&db).unwrap();
    let project = s.create_project(CreateProject {
        logo: None, runner_mapping: None, key: None, request_id: RequestId::new(),
        name: "Delete active".into(), repository: "factory:managed".into(), base_branch: "main".into(),
    }).unwrap();
    configure_models(&s);
    let intent = s.create_ticket(CreateTicket {
        request_id: RequestId::new(), project_id: project.id, title: "Work".into(),
        description: "Wait for input".into(), acceptance_criteria: String::new(), priority: 1,
    }).unwrap();
    let runner = RunnerId::new();
    s.autostart_runner(runner).unwrap();
    s.scheduler_tick().unwrap();
    prepare(&s, &s.pending(runner).unwrap()[0]);
    s.scheduler_tick().unwrap();
    let run = s.pending(runner).unwrap().pop().unwrap();
    s.acknowledge(runner, run.id, AckCommand {
        request_id: RequestId::new(), ownership_epoch: run.ownership_epoch,
    }).unwrap();
    let snapshot = || QuestionSnapshot { ownership_epoch: run.ownership_epoch, questions: vec![ExternalQuestion {
        external_id: "request_form".into(), kind: QuestionKind::Form,
        payload: serde_json::json!({"id":"request_form","sessionID":run.session_id(),"title":"Continue?",
            "fields":[{"key":"answer","type":"string","required":true}]}),
    }] };
    let question = s.sync_questions(runner, run.id, snapshot()).unwrap().pop().unwrap();
    let request = DeleteTask { request_id: RequestId::new() };
    assert_eq!(s.delete_task(intent.id, request).unwrap().status, TicketState::Cancelled);
    drop(s);
    let s = Store::open(&db).unwrap();
    assert_eq!(s.delete_task(intent.id, request).unwrap().status, TicketState::Cancelled);
    assert!(s.task_payload(intent.id).is_err());
    assert!(s.sync_questions(runner, run.id, snapshot()).unwrap().is_empty());
    assert!(s.answer_question(question.id, AnswerQuestion {
        request_id: RequestId::new(), legacy_revision: None, answer: serde_json::json!({"answer":"yes"}),
    }).is_err());
    let control = s.runner_control(runner, run.id, run.ownership_epoch).unwrap().unwrap();
    assert_eq!(control.state, PauseState::Requested);
    assert_eq!(control.command_id, Some(run.id));
    assert_eq!(s.command_payload(run.id).unwrap().state, CommandState::Accepted,
        "Deleting must not fabricate proof that the runner stopped");
}

#[test]
fn saved_publication_requires_the_current_runner_grant_and_does_not_rerun_the_agent() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("db")).unwrap();
    let runner = RunnerId::new();
    let project = s.create_project(CreateProject {
        logo: None, runner_mapping: None, key: None,
        request_id: RequestId::new(), name: "Saved result".into(),
        repository: "factory:managed".into(), base_branch: "main".into(),
    }).unwrap();
    configure_models(&s);
    let intent = s.create_ticket(CreateTicket {
        request_id: RequestId::new(), project_id: project.id, title: "Work".into(),
        description: "Verify publication".into(), acceptance_criteria: String::new(), priority: 1,
    }).unwrap();
    s.autostart_runner(runner).unwrap();
    s.scheduler_tick().unwrap();
    prepare(&s, &s.pending(runner).unwrap()[0]);
    s.scheduler_tick().unwrap();
    let run = s.pending(runner).unwrap().pop().unwrap_or_else(|| {
        panic!("Agent did not queue: {}", s.task_payload(intent.id).unwrap().routing_note)
    });
    agent_reply(&s, &run, &format!("{PUBLICATION_BLOCKED_PREFIX}\nBlocked checkout"));
    // Model a completed local-checkout command without running a runner in this store test.
    let db = rusqlite::Connection::open(dir.path().join("db")).unwrap();
    db.execute("UPDATE projects SET data=json_set(data,'$.repository','/local/project') WHERE id=?1",
        [project.id.to_string()]).unwrap();
    db.execute("UPDATE commands SET data=json_set(data,'$.workspace_spec.setup.repository_source','/local/project') WHERE id=?1",
        [run.id.to_string()]).unwrap();
    let grant = RequestId::new();
    assert!(!s.claim_publication_recovery(runner, run.id, grant).unwrap(), "Require a final archive");
    final_archive(&s, &run);
    assert!(!s.claim_publication_recovery(RunnerId::new(), run.id, grant).unwrap());
    assert!(s.claim_publication_recovery(runner, run.id, grant).unwrap());
    assert!(!s.claim_publication_recovery(runner, run.id, RequestId::new()).unwrap());
    let blocked = s.task_payload(intent.id).unwrap();
    assert!(s.delete_inactive_task(intent.id, DeleteInactiveTask {
        request_id: RequestId::new(), expected_revision: blocked.revision, execution_command: Some(run.id),
    }).is_err());
    let proof = ReportRecoveredPublication {
        request_id: RequestId::new(), grant_id: grant, ownership_epoch: run.ownership_epoch,
        base_branch: "main".into(), snapshot_commit: "b".repeat(40), target_commit: "c".repeat(40),
    };
    let mut wrong = proof.clone();
    wrong.base_branch = "other".into();
    assert!(s.report_recovered_publication(runner, run.id, wrong).is_err());
    s.report_recovered_publication(runner, run.id, proof.clone()).unwrap();
    s.report_recovered_publication(runner, run.id, proof).unwrap();
    let ticket = s.task_payload(intent.id).unwrap();
    assert_eq!(ticket.status, TicketState::Done);
    assert_eq!(ticket.execution_command, Some(run.id));
    assert!(!s.claim_publication_recovery(runner, run.id, RequestId::new()).unwrap());
    assert!(s.pending(runner).unwrap().is_empty());
}

#[test]
fn inactive_delete_accepts_unstarted_and_publication_blocked_intents_but_not_active_work() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("db")).unwrap();
    let project = s.create_project(CreateProject {
        logo: None, runner_mapping: None, key: None, request_id: RequestId::new(),
        name: "Delete inactive".into(), repository: "factory:managed".into(), base_branch: "main".into(),
    }).unwrap();
    configure_models(&s);
    let create = || s.create_ticket(CreateTicket {
        request_id: RequestId::new(), project_id: project.id, title: "Work".into(),
        description: "Preserve worktree".into(), acceptance_criteria: String::new(), priority: 1,
    }).unwrap();
    let delete = |ticket: &Ticket| s.delete_inactive_task(ticket.id, DeleteInactiveTask {
        request_id: RequestId::new(), expected_revision: ticket.revision,
        execution_command: ticket.execution_command,
    });
    let unstarted = create();
    assert_eq!(delete(&unstarted).unwrap().status, TicketState::Cancelled);
    assert!(s.task_payload(unstarted.id).is_err());
    let intent = create();
    let runner = RunnerId::new();
    s.autostart_runner(runner).unwrap();
    s.scheduler_tick().unwrap();
    prepare(&s, &s.pending(runner).unwrap()[0]);
    s.scheduler_tick().unwrap();
    let run = s.pending(runner).unwrap().pop().unwrap();
    let running = s.task_payload(intent.id).unwrap();
    assert_eq!(running.execution_command, Some(run.id));
    assert!(delete(&running).is_err());
    agent_reply(&s, &run, &format!("{PUBLICATION_BLOCKED_PREFIX}\nProject checkout dirty"));
    let blocked = s.task_payload(intent.id).unwrap();
    assert_eq!(blocked.status, TicketState::Blocked);
    assert_eq!(delete(&blocked).unwrap().status, TicketState::Cancelled);
    assert!(s.task_payload(intent.id).is_err());
    assert!(command_publication_blocked(&s.command_payload(run.id).unwrap()));
}

#[test]
fn stopped_intent_recovers_once_in_the_same_archived_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let s = Store::open(&path).unwrap();
    let project = s.create_project(CreateProject {
        logo: None, runner_mapping: None, key: None, request_id: RequestId::new(),
        name: "Continue stopped session".into(), repository: "factory:managed".into(), base_branch: "main".into(),
    }).unwrap();
    configure_models(&s);
    let intent = s.create_ticket(CreateTicket {
        request_id: RequestId::new(), project_id: project.id, title: "Finish the work".into(),
        description: "Keep existing work".into(), acceptance_criteria: String::new(), priority: 1,
    }).unwrap();
    let runner = RunnerId::new();
    s.autostart_runner(runner).unwrap();
    s.scheduler_tick().unwrap();
    prepare(&s, &s.pending(runner).unwrap()[0]);
    s.scheduler_tick().unwrap();
    let first = s.pending(runner).unwrap().pop().unwrap();
    let stopped = |s: &Store, cmd: &RunnerCommand| {
        complete(s, cmd, CommandOutput::Agent { agent: AgentProgress {
            session_id: cmd.session_id(), message_id: agent_message_id(cmd.id),
            phase: AgentPhase::Failed, summary: format!("{SESSION_STOPPED_PREFIX}\nNo final response"),
        }});
    };
    stopped(&s, &first);
    s.scheduler_tick().unwrap();
    assert!(s.pending(runner).unwrap().is_empty());
    final_archive(&s, &first);
    drop(s);
    let s = Store::open(&path).unwrap();
    s.scheduler_tick().unwrap();
    s.scheduler_tick().unwrap();
    let pending = s.pending(runner).unwrap();
    assert_eq!(pending.len(), 1);
    let next = &pending[0];
    assert_eq!(next.session_id(), first.session_id());
    assert_eq!(next.workspace_spec, first.workspace_spec);
    assert_eq!(next.agent_spec.as_ref().unwrap().continuation.as_ref().unwrap().previous_command_id, first.id);
    assert_eq!(s.task_payload(intent.id).unwrap().execution_command, Some(next.id));
    stopped(&s, next);
    final_archive(&s, next);
    s.scheduler_tick().unwrap();
    assert!(s.pending(runner).unwrap().is_empty());
    assert_eq!(s.task_payload(intent.id).unwrap().execution_command, Some(next.id));
}

#[test]
fn reviewer_requeues_the_same_intent_and_session_ahead_of_new_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let s = Store::open(&path).unwrap();
    let project = s.create_project(CreateProject {
        logo: None, runner_mapping: None, key: None, request_id: RequestId::new(),
        name: "Validation".into(), repository: "factory:managed".into(), base_branch: "main".into(),
    }).unwrap();
    s.set_session_limit(project.id, SetSessionLimit {
        request_id: RequestId::new(), expected_revision: project.revision, max_sessions: 1,
    }).unwrap();
    configure_models(&s);
    let create = |s: &Store, text: &str, priority| s.create_ticket(CreateTicket {
        request_id: RequestId::new(), project_id: project.id, title: text.into(),
        description: text.into(), acceptance_criteria: String::new(), priority,
    }).unwrap();
    let intent = create(&s, "Implement all three steps", 1);
    let runner = RunnerId::new();
    s.autostart_runner(runner).unwrap();
    s.scheduler_tick().unwrap();
    prepare(&s, &s.pending(runner).unwrap()[0]);
    s.scheduler_tick().unwrap();
    let first = s.pending(runner).unwrap().pop().unwrap();
    agent_reply(&s, &first, "I implemented step one.");
    let current = |s: &Store, id| s.snapshot().unwrap().tickets.into_iter().find(|t| t.id == id).unwrap();
    assert_eq!(current(&s, intent.id).status, TicketState::Validating);
    let owner = s.snapshot().unwrap().streams.into_iter().find(|w| w.id == first.stream_id).unwrap();
    s.scheduler_tick().unwrap();
    prepare(&s, &s.pending(runner).unwrap()[0]);
    s.scheduler_tick().unwrap();
    let review = s.pending(runner).unwrap().pop().unwrap();
    let spec = review.agent_spec.as_ref().unwrap();
    assert_eq!(spec.role, "reviewer");
    assert_eq!(spec.review_of, Some(intent.id));
    assert!(spec.read_only && spec.images.is_empty() && spec.files.is_empty());
    assert!(spec.prompt.contains("Implement all three steps"));
    assert!(spec.prompt.contains("I implemented step one."));
    agent_reply(&s, &review, r#"{"action":"rework","followup":"Implement steps two and three."}"#);
    s.scheduler_tick().unwrap();
    assert_eq!(current(&s, intent.id).status, TicketState::ReworkQueued);
    assert!(s.pending(runner).unwrap().is_empty(), "Do not resume before the final archive");
    final_archive(&s, &first);
    assert!(s.continue_agent(first.stream_id, ContinueAgent {
        request_id: RequestId::new(), expected_revision: owner.revision,
        previous_command_id: first.id, prompt: "Unrelated work".into(),
    }).is_err(), "Only the queued Reviewer handoff may resume this session");
    let fresh = create(&s, "A high-priority new intent", 3);
    drop(s);
    let s = Store::open(&path).unwrap();
    s.scheduler_tick().unwrap();
    s.scheduler_tick().unwrap();
    let pending = s.pending(runner).unwrap();
    assert_eq!(pending.len(), 1);
    let rework = &pending[0];
    assert_eq!(rework.stream_id, first.stream_id);
    assert_eq!(rework.session_id(), first.session_id());
    assert_eq!(rework.agent_spec.as_ref().unwrap().continuation.as_ref().unwrap().previous_command_id, first.id);
    assert!(rework.agent_spec.as_ref().unwrap().prompt.contains("Implement steps two and three."));
    assert_eq!(current(&s, intent.id).execution_command, Some(rework.id));
    assert_eq!(current(&s, fresh.id).execution_command, None);
    agent_reply(&s, rework, "I implemented steps two and three.");
    s.scheduler_tick().unwrap();
    let reviewer = s.pending(runner).unwrap().into_iter().find(|c| c.agent_spec.as_ref().is_some_and(|a| a.role == "reviewer")).unwrap();
    let prompt = &reviewer.agent_spec.as_ref().unwrap().prompt;
    assert!(prompt.contains("I implemented step one.") && prompt.contains("I implemented steps two and three."));
    agent_reply(&s, &reviewer, "Not a valid verdict");
    s.scheduler_tick().unwrap();
    let blocked = current(&s, intent.id);
    assert_eq!(blocked.status, TicketState::Blocked);
    let retried = s.retry_task(intent.id, RetryTask {
        request_id: RequestId::new(), expected_revision: blocked.revision, previous_command: rework.id,
    }).unwrap();
    assert_eq!(retried.status, TicketState::Validating);
    assert_eq!(retried.execution_command, Some(rework.id), "Retry only the Reviewer, not the Implementer");
    s.scheduler_tick().unwrap();
    let new_review = s.pending(runner).unwrap().into_iter().find(|c| c.agent_spec.as_ref().is_some_and(|a| a.role == "reviewer")).unwrap();
    assert_ne!(new_review.id, reviewer.id);
    assert!(s.snapshot().unwrap().commands.iter().any(|c| c.id == reviewer.id));
    agent_reply(&s, &new_review, r#"{"action":"approve"}"#);
    s.scheduler_tick().unwrap();
    assert_eq!(current(&s, intent.id).status, TicketState::Done);
    assert!(s.task_history(intent.id, None).unwrap().entries.iter().any(|e| e.message.contains("Nacharbeit")));
}

fn decide(s: &Store, c: &RunnerCommand, text: &str) {
    assert_eq!(c.agent_spec.as_ref().unwrap().role, "orchestrator");
    assert!(c.agent_spec.as_ref().unwrap().read_only);
    complete(
        s,
        c,
        CommandOutput::Agent {
            agent: AgentProgress {
                session_id: c.session_id(),
                message_id: agent_message_id(c.id),
                phase: AgentPhase::Succeeded,
                summary: text.into(),
            },
        },
    );
}
fn route_new(s: &Store, task: TicketId) {
    for _ in 0..5 {
        s.scheduler_tick().unwrap();
        let snap = s.snapshot().unwrap();
        if snap
            .streams
            .iter()
            .any(|stream| stream.ticket_ids.contains(&task))
        {
            return;
        }
        let c = snap
            .commands
            .iter()
            .find(|c| c.state == CommandState::Pending)
            .unwrap();
        if c.kind == CommandKind::PrepareWorkspace {
            prepare(s, c);
        } else if c.agent_spec.as_ref().is_some_and(|a| a.role == "reviewer") {
            agent_reply(s, c, r#"{"action":"approve"}"#);
        } else {
            decide(s, c, r#"{"action":"new","reason":"Independent task"}"#);
        }
    }
    panic!("routing did not assign task");
}

fn approve_review(s: &Store, runner: RunnerId) {
    s.scheduler_tick().unwrap();
    let review = s.pending(runner).unwrap().into_iter()
        .find(|c| c.agent_spec.as_ref().is_some_and(|a| a.role == "reviewer")).unwrap();
    agent_reply(s, &review, r#"{"action":"approve"}"#);
    s.scheduler_tick().unwrap();
}

#[test]
fn orchestrated_git_runs_directly_or_separates_the_worker_prompt() {
    for mixed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("store");
        let store = Store::open(&db).unwrap();
        let project = store.create_project(CreateProject {
            key: None, logo: None, runner_mapping: None, request_id: RequestId::new(),
            name: "Release routing".into(), repository: dir.path().join("project").display().to_string(),
            base_branch: "main".into(),
        }).unwrap();
        configure_models(&store);
        let runner = RunnerId::new();
        let original = if mixed { "Implement X, then commit, tag, push" } else { "commit, tag, push" };
        let task = store.save_task(SaveTask {
            request_id: RequestId::new(), id: None, expected_revision: Revision(0),
            project_id: project.id, requested_session: None, prompt: original.into(), images: vec![],
        }).unwrap();
        store.autostart_runner(runner).unwrap();
        store.scheduler_tick().unwrap();
        prepare(&store, &store.pending(runner).unwrap()[0]);
        store.scheduler_tick().unwrap();
        let evaluation = store.pending(runner).unwrap().pop().unwrap();
        assert!(evaluation.agent_spec.as_ref().unwrap().prompt.contains(original));
        decide(&store, &evaluation, &serde_json::json!({"action":"target_git","reason":"Explicit request",
            "worktree_prompt": mixed.then_some("Implement X"),
            "git":{"commit":true,"tag":{"kind":"patch"},"push":true}}).to_string());
        store.scheduler_tick().unwrap();
        let commands = store.pending(runner).unwrap();
        assert_eq!(commands.len(), 1);
        let first = &commands[0];
        assert_eq!(first.kind, if mixed { CommandKind::PrepareWorkspace } else { CommandKind::TargetGit });
        if mixed { prepare(&store, first); }
        drop(store);
        let store = Store::open(&db).unwrap();
        store.scheduler_tick().unwrap();
        let pending = store.pending(runner).unwrap();
        assert_eq!(pending.len(), 1);
        let command = &pending[0];
        if mixed {
            assert_eq!(command.kind, CommandKind::RunAgent);
            let prompt = &command.agent_spec.as_ref().unwrap().prompt;
            assert!(prompt.starts_with("Implement X\n"));
            assert!(!prompt.contains(original));
        } else {
            assert_eq!(first.id, command.id);
            assert!(command.agent_spec.is_none());
        }
        let plan = command.workspace_spec.as_ref().unwrap().target_git.as_ref().unwrap();
        assert!(plan.commit && plan.push);
        assert_eq!(plan.tag, Some(TargetTag::Patch));
        assert!(store.require_target_git_support(runner, command.id, 1, false).is_err());
        assert_eq!(store.snapshot().unwrap().tickets.iter().find(|t| t.id == task.id).unwrap().execution_command, Some(command.id));
    }
}

#[test]
fn committed_intent_wakes_scheduler_and_remapping_moves_future_orchestration() {
    use futures_util::FutureExt;
    let store = Store::open(std::path::Path::new(":memory:")).unwrap();
    let project = store.create_project(CreateProject {
        key: None, logo: None, runner_mapping: None, request_id: RequestId::new(),
        name: "Wakeup".into(), repository: "factory:managed".into(), base_branch: "main".into(),
    }).unwrap();
    let old = RunnerId::new();
    let new = RunnerId::new();
    configure_models(&store);
    let wakeup = store.scheduler_wakeup();
    let submission = |prompt: &str| SaveTask {
        request_id: RequestId::new(), id: None, expected_revision: Revision(0),
        project_id: project.id, requested_session: None, prompt: prompt.into(), images: vec![],
    };
    store.submit_task(SubmitTask {
        task: submission("Draft only"), draft: true, files: vec![], edit_token: None,
    }).unwrap();
    assert!(wakeup.notified().now_or_never().is_none());
    assert!(store.save_task(submission("")).is_err());
    assert!(wakeup.notified().now_or_never().is_none());
    let task = store.save_task(submission("Route this intent")).unwrap();
    assert!(wakeup.notified().now_or_never().is_some());
    assert!(store.snapshot().unwrap().tickets.iter().any(|ticket| ticket.id == task.id));
    store.autostart_runner(old).unwrap();
    store.scheduler_tick().unwrap();
    let prep = store.pending(old).unwrap().into_iter()
        .find(|c| c.kind == CommandKind::PrepareWorkspace).unwrap();
    prepare(&store, &prep);
    let project = store.snapshot().unwrap().projects.into_iter().find(|p| p.id == project.id).unwrap();
    store.update_project(project.id, UpdateProject {
        request_id: RequestId::new(), expected_revision: project.revision,
        runner_mapping: Some(ProjectRunnerMapping::Runner(new)), name: None, key: None,
        repository: None, base_branch: None, logo: None,
    }).unwrap();
    store.scheduler_tick().unwrap();
    assert!(store.pending(old).unwrap().is_empty());
    let prep = store.pending(new).unwrap().into_iter()
        .find(|c| c.kind == CommandKind::PrepareWorkspace).unwrap();
    prepare(&store, &prep);
    store.scheduler_tick().unwrap();
    assert!(store.pending(new).unwrap().iter()
        .any(|c| c.agent_spec.as_ref().is_some_and(|a| a.role == "orchestrator")));
}

#[test]
fn archived_session_with_new_skill_grants_queues_and_sends_to_same_root() {
    let store = Store::open(std::path::Path::new(":memory:")).unwrap();
    let project = store.create_project(CreateProject {
        logo: None,
        runner_mapping: None, key: None, request_id: RequestId::new(),
        name: "Archived".into(), repository: "factory:managed".into(), base_branch: "main".into(),
    }).unwrap();
    configure_models(&store);
    let runner = RunnerId::new();
    let first = store.save_task(SaveTask {
        request_id: RequestId::new(), id: None, expected_revision: Revision(0),
        project_id: project.id, prompt: "Initial request".into(), images: vec![], requested_session: None,
    }).unwrap();
    configure_models(&store);
    store.autostart_runner(runner).unwrap();
    route_new(&store, first.id);
    let prep = store.pending(runner).unwrap().into_iter()
        .find(|c| c.kind == CommandKind::PrepareWorkspace).unwrap();
    prepare(&store, &prep);
    store.scheduler_tick().unwrap();
    let original = store.pending(runner).unwrap().into_iter()
        .find(|c| c.agent_spec.as_ref().is_some_and(|a| !a.read_only)).unwrap();
    assert!(original.agent_spec.as_ref().unwrap().repo_skills.is_none());
    complete(&store, &original, CommandOutput::Agent { agent: AgentProgress {
        session_id: original.session_id(), message_id: agent_message_id(original.id),
        phase: AgentPhase::Succeeded, summary: "Done".into(),
    }});
    store.report_archive(runner, original.id, ReportArchive {
        ownership_epoch: original.ownership_epoch, batch: ArchiveBatch {
            session_id: original.session_id(), revision: 1, captured_unix_ms: 1,
            final_scan_complete: true, messages: vec![],
        },
    }).unwrap();
    approve_review(&store, runner);
    let owner = store.snapshot().unwrap().streams.into_iter()
        .find(|w| w.id == original.stream_id).unwrap();
    store.close_session(original.id, CloseSession {
        request_id: RequestId::new(), expected_stream_revision: owner.revision, reason: "Archived".into(),
    }).unwrap();
    store.set_project_skills(project.id, SetProjectSkills {
        request_id: RequestId::new(), expected_revision: Revision(0),
        repo_skills: vec!["new-skill".into()], factory_skills: vec![],
    }).unwrap();
    let queued = store.prompt_session(original.id, PromptSession {
        request_id: RequestId::new(), prompt: "Follow up in this session".into(), images: vec![], files: vec![],
    }).unwrap();
    assert_eq!(queued.requested_session, Some(original.id));
    assert!(store.session_closure(original.id).unwrap().is_none());
    store.scheduler_tick().unwrap();
    store.scheduler_tick().unwrap();
    let continued = store.pending(runner).unwrap().into_iter()
        .find(|c| c.id != original.id && c.session_root() == original.id).unwrap();
    assert_eq!(continued.session_id(), original.session_id());
    assert_eq!(continued.runner_id, original.runner_id);
    let spec = continued.agent_spec.unwrap();
    assert!(spec.prompt.starts_with("Follow up in this session"));
    assert_eq!(spec.repo_skills, Some(Vec::new()), "Newly approved skills are not inherited");
    assert!(spec.factory_skills.is_empty());
}

#[tokio::test]
async fn fixed_session_waits_then_continues_same_root_and_limit_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let s = Store::open(&path).unwrap();
    let p = s
        .create_project(CreateProject {
            logo: None,
            runner_mapping: None,
            key: None,
            request_id: RequestId::new(),
            name: "Fixed".into(),
            repository: "factory:managed".into(),
            base_branch: "main".into(),
        })
        .unwrap();
    let runner = RunnerId::new();
    configure_models(&s);
    s.autostart_runner(runner).unwrap();
    let task = |text: &str, session| SaveTask {
        request_id: RequestId::new(),
        id: None,
        expected_revision: Revision(0),
        project_id: p.id,
        prompt: text.into(),
        images: vec![],
        requested_session: session,
    };
    let first = s.save_task(task("Build calculator", None)).unwrap();
    configure_models(&s);
    s.autostart_runner(runner).unwrap();
    route_new(&s, first.id);
    let automatic = s.supervisor_policy(p.id).unwrap().unwrap();
    assert!(automatic.enabled);
    assert_eq!(automatic.remaining_evaluations, 12);
    assert_eq!(automatic.interval_seconds, 300);
    let prep = s
        .pending(runner)
        .unwrap()
        .into_iter()
        .find(|c| c.kind == CommandKind::PrepareWorkspace)
        .unwrap();
    prepare(&s, &prep);
    s.scheduler_tick().unwrap();
    let c = s
        .pending(runner)
        .unwrap()
        .into_iter()
        .find(|c| c.kind == CommandKind::RunAgent)
        .unwrap();
    let direct = PromptSession {
        request_id: RequestId::new(), prompt: format!("Add multiplication\n{}End of follow-up.", "Details: ä, ö, ü.\n".repeat(3_000)), images: vec![],
        files: vec![PromptFile::from_bytes("details.txt".into(), b"Direct session details").unwrap()],
    };
    let routing_count = s.snapshot().unwrap().commands.iter()
        .filter(|c| c.agent_spec.as_ref().is_some_and(|a| a.role == "orchestrator")).count();
    let follow = post_prompt(&s, &format!("/v1/commands/{}/prompt", c.id), &direct).await;
    assert_eq!(follow.prompt.as_deref(), Some(direct.prompt.as_str()));
    assert_eq!(follow.session_prompt_request, Some(direct.request_id));
    assert_eq!(follow.requested_session, Some(c.id));
    assert_eq!(follow.project_id, p.id);
    assert_eq!(s.prompt_session(c.id, direct.clone()).unwrap().id, follow.id);
    let controller = s.snapshot().unwrap().commands.into_iter()
        .find(|c| c.agent_spec.as_ref().is_some_and(|a| a.read_only)).unwrap();
    assert!(s.prompt_session(controller.id, PromptSession { request_id: RequestId::new(), ..direct.clone() }).is_err());
    assert!(s.prompt_session(CommandId::new(), PromptSession { request_id: RequestId::new(), ..direct.clone() }).is_err());
    s.scheduler_tick().unwrap();
    assert!(!s
        .snapshot()
        .unwrap()
        .streams
        .iter()
        .any(|s| s.ticket_ids.contains(&follow.id)));
    complete(
        &s,
        &c,
        CommandOutput::Agent {
            agent: AgentProgress {
                session_id: c.session_id(),
                message_id: agent_message_id(c.id),
                phase: AgentPhase::Succeeded,
                summary: "Calculator done".into(),
            },
        },
    );
    s.scheduler_tick().unwrap();
    assert!(!s
        .snapshot()
        .unwrap()
        .streams
        .iter()
        .any(|s| s.ticket_ids.contains(&follow.id)));
    s.report_archive(
        runner,
        c.id,
        ReportArchive {
            ownership_epoch: 1,
            batch: ArchiveBatch {
                session_id: c.session_id(),
                revision: 1,
                captured_unix_ms: 1,
                final_scan_complete: true,
                messages: vec![],
            },
        },
    )
    .unwrap();
    approve_review(&s, runner);
    let continued = s
        .pending(runner)
        .unwrap()
        .into_iter()
        .find(|a| a.kind == CommandKind::RunAgent)
        .unwrap();
    assert_eq!(continued.session_root(), c.id);
    assert_eq!(continued.stream_id, c.stream_id);
    assert_eq!(continued.runner_id, c.runner_id);
    assert_eq!(continued.agent_spec.as_ref().unwrap().files, direct.files);
    assert_eq!(continued.agent_spec.as_ref().unwrap().local_mcps, c.agent_spec.as_ref().unwrap().local_mcps);
    assert_eq!(s.snapshot().unwrap().commands.iter()
        .filter(|c| c.agent_spec.as_ref().is_some_and(|a| a.role == "orchestrator")).count(), routing_count);
    assert!(continued
        .agent_spec
        .as_ref()
        .unwrap()
        .prompt
        .starts_with(&direct.prompt));
    assert_eq!(
        s.snapshot()
            .unwrap()
            .tickets
            .iter()
            .find(|t| t.id == first.id)
            .unwrap()
            .execution_command,
        Some(c.id)
    );
    complete(
        &s,
        &continued,
        CommandOutput::Agent {
            agent: AgentProgress {
                session_id: continued.session_id(),
                message_id: agent_message_id(continued.id),
                phase: AgentPhase::Succeeded,
                summary: "Multiplication done".into(),
            },
        },
    );
    s.report_archive(
        runner,
        continued.id,
        ReportArchive {
            ownership_epoch: 1,
            batch: ArchiveBatch {
                session_id: continued.session_id(),
                revision: 1,
                captured_unix_ms: 1,
                final_scan_complete: true,
                messages: vec![],
            },
        },
    )
    .unwrap();
    let automatic = s.save_task(task("Add division", None)).unwrap();
    approve_review(&s, runner);
    let routing = s
        .pending(runner)
        .unwrap()
        .into_iter()
        .find(|c| {
            c.agent_spec
                .as_ref()
                .is_some_and(|a| a.role == "orchestrator")
        })
        .unwrap();
    assert!(routing
        .agent_spec
        .as_ref()
        .unwrap()
        .prompt
        .contains(&c.id.to_string()));
    decide(
        &s,
        &routing,
        &serde_json::json!({"action":"session","session_id":c.id,"reason":"Continue calculator"})
            .to_string(),
    );
    s.scheduler_tick().unwrap();
    let reuse = s
        .pending(runner)
        .unwrap()
        .into_iter()
        .find(|c| c.agent_spec.as_ref().is_some_and(|a| !a.read_only))
        .unwrap();
    assert_eq!(reuse.session_root(), c.id);
    assert_eq!(
        reuse
            .agent_spec
            .as_ref()
            .unwrap()
            .continuation
            .as_ref()
            .unwrap()
            .previous_command_id,
        continued.id
    );
    assert_eq!(
        s.snapshot()
            .unwrap()
            .tickets
            .iter()
            .find(|t| t.id == automatic.id)
            .unwrap()
            .execution_command,
        Some(reuse.id)
    );
    let p = s.snapshot().unwrap().projects[0].clone();
    let limit = SetSessionLimit {
        request_id: RequestId::new(),
        expected_revision: p.revision,
        max_sessions: 3,
    };
    let changed = s.set_session_limit(p.id, limit.clone()).unwrap();
    assert_eq!(changed.max_sessions, 3);
    assert_eq!(
        s.set_session_limit(p.id, limit).unwrap().revision,
        changed.revision
    );
    assert!(s
        .set_session_limit(
            p.id,
            SetSessionLimit {
                request_id: RequestId::new(),
                expected_revision: p.revision,
                max_sessions: 2
            }
        )
        .is_err());
    drop(s);
    let s = Store::open(&path).unwrap();
    assert_eq!(s.scheduler_policy(p.id).unwrap().unwrap().max_parallel, 3);
    assert_eq!(s.snapshot().unwrap().projects[0].max_sessions, 3);
}

#[test]
fn orchestrator_wait_and_invalid_decisions_do_not_start_work_or_loop() {
    for answer in [
        r#"{"action":"wait","reason":"Need a related result"}"#,
        r#"{"action":"discard","reason":"Repeated text without an intent"}"#,
        r#"{"action":"session","session_id":"00000000-0000-0000-0000-000000000000","reason":"invalid candidate"}"#,
        "not JSON",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let s = Store::open(&path).unwrap();
        let p = s
            .create_project(CreateProject {
                logo: None,
                runner_mapping: None,
                key: None,
                request_id: RequestId::new(),
                name: "Waiting".into(),
                repository: "factory:managed".into(),
                base_branch: "main".into(),
            })
            .unwrap();
        let runner = RunnerId::new();
        configure_models(&s);
        s.autostart_runner(runner).unwrap();
        let t = s
            .save_task(SaveTask {
                request_id: RequestId::new(),
                id: None,
                expected_revision: Revision(0),
                project_id: p.id,
                prompt: "Follow up".into(),
                images: vec![],
                requested_session: None,
            })
            .unwrap();
        configure_models(&s);
        s.autostart_runner(runner).unwrap();
        s.scheduler_tick().unwrap();
        prepare(&s, &s.pending(runner).unwrap()[0]);
        s.scheduler_tick().unwrap();
        let c = s.pending(runner).unwrap()[0].clone();
        decide(&s, &c, answer);
        s.scheduler_tick().unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        // Simulate the old retry timer expiring, also for malformed replies
        // after they have reached the bounded attempt limit.
        let terminal = answer.contains("\"wait\"") || answer.contains("\"discard\"");
        if !terminal {
            let count: u32 = db.query_row("SELECT json_extract(data,'$.attempts') FROM task_routing WHERE ticket_id=?1",
                [t.id.to_string()], |r| r.get(0)).unwrap();
            assert_eq!(count, 1);
        }
        db.execute("UPDATE task_routing SET data=json_set(data,'$.after_ms',0,'$.attempts',?2) WHERE ticket_id=?1",
            rusqlite::params![t.id.to_string(), if terminal { 1 } else { 3 }]).unwrap();
        drop(s);
        let s = Store::open(&path).unwrap();
        for _ in 0..5 {
            s.scheduler_tick().unwrap();
        }
        let snap = s.snapshot().unwrap();
        assert_eq!(snap.commands.len(), 2);
        assert!(!snap
            .streams
            .iter()
            .any(|stream| stream.ticket_ids.contains(&t.id)));
        assert!(!snap.tickets[0].routing_note.is_empty());
        if answer.contains("\"discard\"") {
            assert_eq!(snap.tickets[0].status, TicketState::Cancelled);
            assert!(snap.tickets[0].routing_note.contains("Repeated text without an intent"));
            assert_eq!(snap.tickets[0].prompt.as_deref(), Some("Follow up"));
            assert!(s.task_history(t.id,None).unwrap().entries.iter().any(|e| e.message.contains("Intent verworfen")));
        } else {
            assert_eq!(snap.tickets[0].status, TicketState::Backlog);
        }
    }
}

#[test]
fn legacy_missing_folder_failure_gets_one_fenced_replacement_and_scheduler_continues() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let s = Store::open(&path).unwrap();
    let p = s
        .create_project(CreateProject {
            logo: None,
            runner_mapping: None,
            key: None,
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
    configure_models(&s);
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
            requested_session: None,
            request_id: RequestId::new(),
            id: None,
            expected_revision: Revision(0),
            project_id: p.id,
            prompt: "Follow-up".into(),
            images: vec![],
        })
        .unwrap();
    s.scheduler_tick().unwrap();
    route_new(&s, next.id);
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
            logo: None,
            runner_mapping: None,
            key: None,
            request_id: RequestId::new(),
            name: "Auto".into(),
            repository: "factory:managed".into(),
            base_branch: "main".into(),
        })
        .unwrap();
    let image = PromptImage::from_bytes("example.png".into(), b"\x89PNG\r\n\x1a\nfixture").unwrap();
    let b = SaveTask {
        requested_session: None,
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
    configure_models(&s);
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
    let routing = commands
        .iter()
        .find(|c| c.kind == CommandKind::RunAgent)
        .unwrap();
    assert_eq!(
        routing.agent_spec.as_ref().unwrap().images,
        vec![image.clone()]
    );
    decide(
        &s,
        routing,
        r#"{"action":"new","reason":"Build first task"}"#,
    );
    s.scheduler_tick().unwrap();
    let prep = s
        .pending(runner)
        .unwrap()
        .into_iter()
        .find(|c| c.kind == CommandKind::PrepareWorkspace)
        .unwrap();
    prepare(&s, &prep);
    s.scheduler_tick().unwrap();
    let c = s
        .pending(runner)
        .unwrap()
        .into_iter()
        .find(|c| c.kind == CommandKind::RunAgent)
        .unwrap();
    let spec = c.agent_spec.as_ref().unwrap();
    assert!(spec.prompt.starts_with(&b.prompt));
    assert_eq!(spec.images, vec![image]);
    assert!(s.require_image_support(runner, c.id, 1, false).is_err());
    assert!(s.require_image_support(runner, c.id, 1, true).is_ok());
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
    route_new(&s, next.id);
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
