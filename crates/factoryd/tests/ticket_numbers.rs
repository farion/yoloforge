use factory_core::*;
use factory_protocol::*;
use factoryd::store::{Store,StoreError};

fn project(store:&Store,key:Option<&str>)->Project {
    store.create_project(CreateProject {
        logo: None,
        runner_mapping: None,
        key:key.map(str::to_string),request_id:RequestId::new(),name:"WorldOfAI".into(),
        repository:"factory:managed".into(),base_branch:"main".into(),
    }).unwrap()
}
fn task(project_id:ProjectId)->SaveTask {
    SaveTask {request_id:RequestId::new(),id:None,expected_revision:Revision(0),
        project_id,prompt:"Unveränderter Auftragstext".into(),images:vec![],requested_session:None}
}
fn technical(store:&Store,project_id:ProjectId)->Ticket {
    store.create_ticket(CreateTicket {
        request_id:RequestId::new(),project_id,title:"Technisch".into(),description:"Text".into(),
        acceptance_criteria:"Prüfen".into(),priority:1,
    }).unwrap()
}

#[test]
fn numbers_are_project_scoped_stable_for_edits_drafts_and_request_replays() {
    let dir=tempfile::tempdir().unwrap();
    let path=dir.path().join("numbers.sqlite");
    let store=Store::open(&path).unwrap();
    let p=project(&store,Some("woai"));
    assert_eq!(p.key,"WOAI");
    let submission=SubmitTask {task:task(p.id),draft:true,files:vec![],edit_token:None};
    let first=store.submit_task(submission.clone()).unwrap();
    assert_eq!(first.key,"WOAI-1");
    assert_eq!(first.prompt.as_deref(),Some("Unveränderter Auftragstext"));
    assert_eq!(store.submit_task(submission.clone()).unwrap(),first);
    let mut edited=submission.clone();
    edited.task.request_id=RequestId::new();
    edited.task.id=Some(first.id);
    edited.task.expected_revision=first.revision;
    edited.task.prompt="Bearbeiteter Text".into();
    let edited=store.submit_task(edited).unwrap();
    assert_eq!(edited.key,first.key);
    let second=technical(&store,p.id);
    assert_eq!(second.key,"WOAI-2");
    let p2=project(&store,Some("TEST"));
    assert_eq!(store.save_task(task(p2.id)).unwrap().key,"TEST-1");
    drop(store);
    let store=Store::open(&path).unwrap();
    assert_eq!(store.submit_task(submission).unwrap().key,"WOAI-1");
    assert_eq!(store.save_task(task(p.id)).unwrap().key,"WOAI-3");
    let mut invalid=task(p.id);
    invalid.prompt.clear();
    assert!(store.save_task(invalid).is_err());
    assert_eq!(store.save_task(task(p.id)).unwrap().key,"WOAI-4","Failed writes must not consume numbers");
}

#[test]
fn automatic_keys_are_unique_and_explicit_keys_are_validated() {
    let dir=tempfile::tempdir().unwrap();
    let store=Store::open(&dir.path().join("keys.sqlite")).unwrap();
    let first=project(&store,None);
    let second=project(&store,None);
    assert_eq!(first.key,"WOAI");
    assert_eq!(second.key,"WOAI2");
    for key in ["WOAI","woai","bad-key","1BAD","ä","TOOLONGPROJECTKEY"] {
        assert!(matches!(store.create_project(CreateProject {
            logo: None,
            runner_mapping: None,
            key:Some(key.into()),request_id:RequestId::new(),name:"Other".into(),
            repository:"factory:managed".into(),base_branch:"main".into(),
        }),Err(StoreError::Invalid(_))),"{key}");
    }
}

#[test]
fn a_failed_event_rolls_back_the_ticket_and_counter_together() {
    let dir=tempfile::tempdir().unwrap();
    let path=dir.path().join("rollback.sqlite");
    let store=Store::open(&path).unwrap();
    let p=project(&store,Some("TEST"));
    let db=rusqlite::Connection::open(path).unwrap();
    db.execute_batch("CREATE TRIGGER numbering_failure BEFORE INSERT ON events
        WHEN NEW.kind IN ('task.saved','ticket.created')
        BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
    assert!(store.save_task(task(p.id)).is_err());
    assert!(store.create_ticket(CreateTicket {request_id:RequestId::new(),project_id:p.id,
        title:"Rollback".into(),description:"Text".into(),acceptance_criteria:"Test".into(),priority:1}).is_err());
    assert!(store.snapshot().unwrap().tickets.is_empty());
    db.execute_batch("DROP TRIGGER numbering_failure;").unwrap();
    assert_eq!(store.save_task(task(p.id)).unwrap().key,"TEST-1");
}

#[test]
fn prefix_changes_keep_sequence_and_reserve_old_keys_without_touching_prompts() {
    let dir=tempfile::tempdir().unwrap();
    let store=Store::open(&dir.path().join("rename.sqlite")).unwrap();
    let p=project(&store,Some("OLD"));
    let first=store.save_task(task(p.id)).unwrap();
    assert_eq!(technical(&store,p.id).key,"OLD-2");
    let body=UpdateProject {logo:None,runner_mapping:None,key:Some("WOAI".into()),request_id:RequestId::new(),
        expected_revision:p.revision,name:None,repository:None,base_branch:None};
    let renamed=store.update_project(p.id,body.clone()).unwrap();
    assert_eq!(renamed.key,"WOAI");
    assert_eq!(store.update_project(p.id,body).unwrap(),renamed);
    let snapshot=store.snapshot().unwrap();
    let changed=snapshot.tickets.iter().find(|ticket|ticket.id==first.id).unwrap();
    assert_eq!(changed.key,"WOAI-1");
    assert_eq!(changed.prompt,first.prompt);
    assert_eq!(store.save_task(task(p.id)).unwrap().key,"WOAI-3");
    assert!(store.create_project(CreateProject {logo:None,runner_mapping:None,key:Some("OLD".into()),request_id:RequestId::new(),
        name:"Other".into(),repository:"factory:managed".into(),base_branch:"main".into()}).is_err());
}

#[test]
fn concurrent_connections_never_allocate_the_same_number_and_replay_once() {
    let dir=tempfile::tempdir().unwrap();
    let path=dir.path().join("concurrent.sqlite");
    let store=Store::open(&path).unwrap();
    let p=project(&store,Some("TEST"));
    let shared=task(p.id);
    let mut threads=vec![];
    for index in 0..12 {
        let path=path.clone();
        let body=if index<4 {shared.clone()}else{task(p.id)};
        threads.push(std::thread::spawn(move||Store::open(&path).unwrap().save_task(body).unwrap()));
    }
    let tickets:Vec<_>=threads.into_iter().map(|thread|thread.join().unwrap()).collect();
    assert!(tickets[..4].iter().all(|ticket|ticket==&tickets[0]));
    let mut numbers:Vec<_>=store.snapshot().unwrap().tickets.iter()
        .map(|ticket|ticket.key.rsplit_once('-').unwrap().1.parse::<u64>().unwrap()).collect();
    numbers.sort();
    assert_eq!(numbers,(1..=9).collect::<Vec<_>>());
}

#[test]
fn old_databases_get_deterministic_numbers_once_and_continue_after_restart() {
    let dir=tempfile::tempdir().unwrap();
    let path=dir.path().join("migration.sqlite");
    let store=Store::open(&path).unwrap();
    let p=project(&store,None);
    let first=technical(&store,p.id);
    let second=technical(&store,p.id);
    drop(store);
    let db=rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP INDEX projects_ticket_key;DROP INDEX tickets_human_key;
        DROP TABLE ticket_counters;DROP TABLE project_key_aliases;
        UPDATE projects SET data=json_remove(data,'$.key');
        UPDATE tickets SET data=json_remove(data,'$.key');PRAGMA user_version=33;").unwrap();
    db.execute("UPDATE tickets SET data=json_set(data,'$.created_unix_ms',30) WHERE id=?1",
        [first.id.to_string()]).unwrap();
    db.execute("UPDATE tickets SET data=json_set(data,'$.created_unix_ms',10) WHERE id=?1",
        [second.id.to_string()]).unwrap();
    drop(db);
    let store=Store::open(&path).unwrap();
    let snapshot=store.snapshot().unwrap();
    assert_eq!(snapshot.tickets.iter().find(|t|t.id==second.id).unwrap().key,"WOAI-1");
    assert_eq!(snapshot.tickets.iter().find(|t|t.id==first.id).unwrap().key,"WOAI-2");
    assert_eq!(snapshot.tickets.iter().find(|t|t.id==first.id).unwrap().revision,first.revision);
    drop(store);
    let store=Store::open(&path).unwrap();
    assert_eq!(technical(&store,p.id).key,"WOAI-3");
    let db=rusqlite::Connection::open(path).unwrap();
    assert_eq!(db.query_row("PRAGMA user_version",[],|row|row.get::<_,u32>(0)).unwrap(),factoryd::store::SCHEMA_VERSION);
}
