use factory_core::*;
use factory_protocol::*;
use factoryd::store::{Store, StoreError};

#[test]
fn factory_projects_register_independent_revision_guarded_runner_mirrors() {
    let store = Store::open(std::path::Path::new(":memory:")).unwrap();
    let project = store.create_project(CreateProject {
        logo: None,
        runner_mapping: Some(ProjectRunnerMapping::Automatic),
        key: None,
        request_id: RequestId::new(),
        name: "Remote truth".into(),
        repository: "factory:managed".into(),
        base_branch: "main".into(),
    }).unwrap();
    let first = RunnerId::new();
    let second = RunnerId::new();
    let project = store.set_project_mirror(project.id, SetProjectMirror {
        request_id: RequestId::new(), expected_revision: project.revision,
        runner_id: first, path: "/home/first/project".into(),
    }).unwrap();
    let project = store.set_project_mirror(project.id, SetProjectMirror {
        request_id: RequestId::new(), expected_revision: project.revision,
        runner_id: second, path: r"C:\Users\second\project".into(),
    }).unwrap();
    assert_eq!(project.mirrors[&first].path, "/home/first/project");
    assert_eq!(project.mirrors[&second].path, r"C:\Users\second\project");
    assert!(matches!(store.set_project_mirror(project.id, SetProjectMirror {
        request_id: RequestId::new(), expected_revision: Revision(1),
        runner_id: first, path: String::new(),
    }), Err(StoreError::RevisionConflict(_))));
    let project = store.set_project_mirror(project.id, SetProjectMirror {
        request_id: RequestId::new(), expected_revision: project.revision,
        runner_id: first, path: String::new(),
    }).unwrap();
    assert!(!project.mirrors.contains_key(&first));
    assert!(project.mirrors.contains_key(&second));

    let local = store.create_project(CreateProject {
        logo: None, runner_mapping: None, key: None, request_id: RequestId::new(),
        name: "Local".into(), repository: "/local/project".into(), base_branch: "main".into(),
    }).unwrap();
    assert!(matches!(store.set_project_mirror(local.id, SetProjectMirror {
        request_id: RequestId::new(), expected_revision: local.revision,
        runner_id: first, path: "/mirror".into(),
    }), Err(StoreError::Invalid(_))));
}
