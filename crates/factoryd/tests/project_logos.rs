use factory_core::*;
use factory_protocol::*;
use factoryd::store::{Store, StoreError};
use serde_json::json;

fn logo(color: u8) -> ProjectLogo {
    let mut bytes = vec![];
    {
        let mut encoder = png::Encoder::new(&mut bytes, 128, 128);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![color; 128 * 128 * 4])
            .unwrap();
    }
    ProjectLogo::from_png(&bytes).unwrap()
}

fn create(name: &str, image: Option<ProjectLogo>) -> CreateProject {
    serde_json::from_value(json!({
        "request_id": RequestId::new(), "name": name, "repository": "/repo",
        "base_branch": "main", "logo": image,
    }))
    .unwrap()
}

fn change(project: &Project, image: Option<ProjectLogo>) -> UpdateProject {
    serde_json::from_value(json!({
        "request_id": RequestId::new(), "expected_revision": project.revision,
        "logo": { "image": image },
    }))
    .unwrap()
}

#[test]
fn logo_is_optional_durable_shared_and_explicitly_replaceable_or_removable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("factory.db");
    let store = Store::open(&path).unwrap();
    let plain = store.create_project(create("Plain", None)).unwrap();
    assert!(plain.logo.is_none());
    assert!(serde_json::to_value(&plain).unwrap().get("logo").is_none());
    let first = logo(80);
    let request = create("Artwork", Some(first.clone()));
    let project = store.create_project(request.clone()).unwrap();
    assert_eq!(store.create_project(request.clone()).unwrap(), project);
    assert_eq!(project.logo.as_ref(), Some(&first));

    // Older clients updating other metadata never clear the image.
    let renamed = store.update_project(project.id, serde_json::from_value(json!({
        "request_id": RequestId::new(), "expected_revision": project.revision, "name": "Renamed",
    })).unwrap()).unwrap();
    assert_eq!(renamed.logo, Some(first));
    assert_eq!(
        store
            .snapshot()
            .unwrap()
            .projects
            .iter()
            .find(|p| p.id == project.id)
            .unwrap(),
        &renamed
    );
    drop(store);

    let store = Store::open(&path).unwrap();
    assert_eq!(store.create_project(request).unwrap(), project);
    let second = logo(160);
    let replacement = change(&renamed, Some(second.clone()));
    let replaced = store
        .update_project(project.id, replacement.clone())
        .unwrap();
    assert_eq!(replaced.logo, Some(second));
    assert_eq!(
        store.update_project(project.id, replacement).unwrap(),
        replaced
    );
    assert!(matches!(
        store.update_project(project.id, change(&renamed, None)),
        Err(StoreError::RevisionConflict(_))
    ));
    let removal = change(&replaced, None);
    let removed = store.update_project(project.id, removal.clone()).unwrap();
    assert!(removed.logo.is_none());
    assert_eq!(store.update_project(project.id, removal).unwrap(), removed);
    drop(store);
    assert!(Store::open(&path)
        .unwrap()
        .snapshot()
        .unwrap()
        .projects
        .iter()
        .find(|p| p.id == project.id)
        .unwrap()
        .logo
        .is_none());
}

#[test]
fn invalid_artwork_is_rejected_without_mutating_the_project_or_event_log() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("factory.db")).unwrap();
    let invalid = ProjectLogo {
        png_base64: "not an image".into(),
    };
    assert!(matches!(
        store.create_project(create("Invalid", Some(invalid.clone()))),
        Err(StoreError::Invalid(_))
    ));
    assert!(store.snapshot().unwrap().projects.is_empty());
    let project = store
        .create_project(create("Valid", Some(logo(80))))
        .unwrap();
    let cursor = store.snapshot().unwrap().cursor;
    assert!(matches!(
        store.update_project(project.id, change(&project, Some(invalid))),
        Err(StoreError::Invalid(_))
    ));
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.cursor, cursor);
    assert_eq!(snapshot.projects[0], project);
}
