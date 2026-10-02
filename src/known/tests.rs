use super::*;

#[test]
fn a_record_round_trips_and_forgets_by_name() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("deployments.yaml");
    let mut known = BTreeMap::new();
    known.insert("demo-1a2b3c".to_string(), PathBuf::from("/srv/demo"));
    known.insert("hello-4d5e6f".to_string(), PathBuf::from("/srv/hello"));
    write(&path, &known).unwrap();
    assert_eq!(load_from(&path), known);

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains('#'), "keys only: {text}");
}

#[test]
fn an_unreadable_record_is_an_empty_one() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("deployments.yaml");
    assert!(load_from(&path).is_empty());
    std::fs::write(&path, "[not, a, map").unwrap();
    assert!(load_from(&path).is_empty());
}

#[test]
fn a_deployment_is_gone_only_when_that_can_be_proven() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("demo");
    // Deleted from a parent that is still there.
    assert_eq!(presence("demo-1a2b3c", &dir), Presence::Gone);

    let mut project = Project {
        dir: dir.clone(),
        state: Default::default(),
    };
    project.state.compose_project = "demo-1a2b3c".to_string();
    project.save().unwrap();
    assert_eq!(presence("demo-1a2b3c", &dir), Presence::Present);
    // `init --force` wrote a new deployment over it: the old name is gone.
    assert_eq!(presence("demo-999999", &dir), Presence::Gone);
}

#[test]
fn a_typo_in_another_state_file_keeps_the_deployment() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("demo");
    let mut project = Project {
        dir: dir.clone(),
        state: Default::default(),
    };
    project.state.compose_project = "demo-1a2b3c".to_string();
    project.save().unwrap();
    std::fs::write(
        dir.join(".chaps").join("components.yaml"),
        "chap_core: [oops",
    )
    .unwrap();
    assert!(Project::load(&dir).is_err(), "the typo breaks a full load");
    assert_eq!(presence("demo-1a2b3c", &dir), Presence::Present);
}

#[test]
fn an_unreadable_name_or_a_missing_parent_is_not_proof() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("demo");
    std::fs::create_dir_all(dir.join(".chaps")).unwrap();
    std::fs::write(
        dir.join(".chaps").join("project.yaml"),
        "compose_project: [oops",
    )
    .unwrap();
    assert!(matches!(presence("demo-1a2b3c", &dir), Presence::Unsure(_)));

    let unmounted = temp.path().join("Volumes").join("Ext").join("demo");
    assert!(matches!(
        presence("demo-1a2b3c", &unmounted),
        Presence::Unsure(_)
    ));
}
