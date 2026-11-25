use factory_core::*;
use factory_protocol::{directs::*, *};
use factoryd::store::Store;

#[test]
fn direct_turn_is_durable_and_archiving_waits_for_the_answer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = Store::open(&path).unwrap();
    let runner = RunnerId::new();
    let project = store.create_project(CreateProject {
        request_id: RequestId::new(), key: None, logo: None, name: "Direct test".into(),
        repository: "/tmp/direct-project".into(), base_branch: "main".into(),
        runner_mapping: Some(ProjectRunnerMapping::Runner(runner)),
    }).unwrap();
    store.update_profile(project.id, UpdateProfile {
        request_id: RequestId::new(), expected_revision: project.revision,
        profile: ExecutionProfile {
            provider_id: "provider".into(), model_id: "model".into(), ..Default::default()
        },
    }).unwrap();
    let create = CreateDirect { request_id: RequestId::new(), project_id: project.id };
    let direct = store.create_direct(create.clone()).unwrap();
    assert_eq!(store.create_direct(create).unwrap().id, direct.id);
    assert!(store.pending(runner).unwrap().is_empty());
    let first = PromptDirect {
        request_id: RequestId::new(), prompt: "What should we build?".into(),
        selection: Some(DirectSelection { runner_id: runner, model: None, optional_mcps: None }),
        images: vec![], files: vec![],
    };
    let prepare = store.prompt_direct(direct.id, first.clone()).unwrap().current_command_id.unwrap();
    assert_eq!(store.prompt_direct(direct.id, first.clone()).unwrap().current_command_id, Some(prepare));
    assert!(store.prompt_direct(direct.id, PromptDirect { request_id: RequestId::new(), ..first }).is_err());
    let archive = |archived| SetDirectArchived { request_id: RequestId::new(), archived };
    assert_eq!(store.set_direct_archived(direct.id, archive(true)).unwrap().state, DirectState::Archiving);
    drop(store);
    let store = Store::open(&path).unwrap();
    let command = store.pending(runner).unwrap().pop().unwrap();
    assert_eq!(command.id, prepare);
    let ack = || AckCommand { request_id: RequestId::new(), ownership_epoch: command.ownership_epoch };
    store.acknowledge(runner, prepare, ack()).unwrap();
    store.complete(runner, prepare, CommandResult {
        request_id: RequestId::new(), ownership_epoch: command.ownership_epoch,
        result: CommandOutput::Workspace { workspace: Workspace {
            path: "/tmp/direct-workspace".into(), branch: command.workspace_spec.unwrap().branch,
            base_commit: "a".repeat(40),
        }},
    }).unwrap();
    let agent = store.pending(runner).unwrap().pop().expect("Preparation should dispatch the first turn");
    assert_eq!(agent.agent_spec.as_ref().unwrap().role, ROLE);
    assert!(!agent.agent_spec.as_ref().unwrap().read_only);
    assert_eq!(agent.agent_spec.as_ref().unwrap().prompt, "What should we build?");
    let state = store.list_directs(project.id).unwrap().pop().unwrap();
    assert_eq!(state.root_command_id, Some(agent.id));
    assert_eq!(state.state, DirectState::Archiving);
    assert!(store.snapshot().unwrap().tickets.is_empty());
    assert_eq!(store.set_direct_archived(direct.id, archive(false)).unwrap().state, DirectState::Open);
    assert_eq!(store.set_direct_archived(direct.id, archive(true)).unwrap().state, DirectState::Archiving);
    store.acknowledge(runner, agent.id, ack()).unwrap();
    let proposal = |id: &str, enqueue| ExternalQuestion {
        kind: QuestionKind::Permission,
        external_id: id.into(),
        payload: serde_json::json!({
            "id": id, "sessionID": agent.session_id(), "action": INTENT_APPROVAL, "resources": [id],
            "metadata": {"direct_intent":{"title":"Implement feature","prompt":"Use the agreed interface.","enqueue":enqueue}}
        }),
    };
    let questions = QuestionSnapshot {
        ownership_epoch: agent.ownership_epoch,
        questions: vec![proposal("per_draft", false), proposal("per_enqueue", true)],
    };
    let observe = || store.sync_questions(runner, agent.id, questions.clone()).unwrap();
    let proposed = observe();
    assert_eq!(proposed[0].state, QuestionState::AnswerQueued);
    assert_eq!(proposed[1].state, QuestionState::Pending);
    observe();
    let tickets = store.snapshot().unwrap().tickets;
    assert_eq!(tickets.len(), 1);
    assert!(tickets[0].draft);
    let approve = |decision| AnswerQuestion {
        request_id: RequestId::new(), legacy_revision: None,
        answer: serde_json::json!({"decision":decision}),
    };
    assert!(store.answer_question(proposed[1].id, approve("always")).is_err());
    assert_eq!(store.snapshot().unwrap().tickets.len(), 1);
    let answer = approve("once");
    store.answer_question(proposed[1].id, answer.clone()).unwrap();
    store.answer_question(proposed[1].id, answer).unwrap();
    let tickets = store.snapshot().unwrap().tickets;
    assert_eq!(tickets.len(), 2);
    assert_eq!(tickets.iter().filter(|t| !t.draft).count(), 1);
    assert!(tickets.iter().all(|t| t.project_id == project.id && t.requested_session.is_none()));
    store.report_archive(runner, agent.id, ReportArchive {
        ownership_epoch: agent.ownership_epoch,
        batch: ArchiveBatch {
            session_id: agent.session_id(), revision: 1, captured_unix_ms: 1,
            final_scan_complete: true, messages: vec![],
        },
    }).unwrap();
    store.complete(runner, agent.id, CommandResult {
        request_id: RequestId::new(), ownership_epoch: agent.ownership_epoch,
        result: CommandOutput::Agent { agent: AgentProgress {
            session_id: agent.session_id(), message_id: agent_message_id(agent.id),
            phase: AgentPhase::Succeeded, summary: "Done".into(),
        }},
    }).unwrap();
    assert_eq!(store.list_directs(project.id).unwrap()[0].state, DirectState::Archived);
    store.set_direct_archived(direct.id, archive(false)).unwrap();
    let next = store.prompt_direct(direct.id, PromptDirect {
        request_id: RequestId::new(), prompt: "And now?".into(),
        selection: None, images: vec![], files: vec![],
    }).unwrap();
    let next_agent = store.pending(runner).unwrap().into_iter().find(|c| Some(c.id) == next.current_command_id).unwrap();
    assert_eq!(next_agent.session_id(), agent.session_id());
}

#[test]
fn empty_direct_can_be_archived_and_reactivated_without_commands() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("db")).unwrap();
    let project = store.create_project(CreateProject {
        request_id: RequestId::new(), key: None, logo: None, name: "Empty Direct".into(),
        repository: "/tmp/direct-project".into(), base_branch: "main".into(), runner_mapping: None,
    }).unwrap();
    let direct = store.create_direct(CreateDirect { request_id: RequestId::new(), project_id: project.id }).unwrap();
    for (archived, expected) in [(true, DirectState::Archived), (false, DirectState::Open)] {
        assert_eq!(store.set_direct_archived(direct.id, SetDirectArchived {
            request_id: RequestId::new(), archived,
        }).unwrap().state, expected);
    }
    let snapshot = store.snapshot().unwrap();
    assert!(snapshot.commands.is_empty() && snapshot.tickets.is_empty());
    assert_eq!(snapshot.directs[0].id, direct.id);
}
