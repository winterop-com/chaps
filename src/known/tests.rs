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
fn a_deployment_is_present_only_while_its_directory_holds_that_name() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("demo");
    assert!(!is_present("demo-1a2b3c", &dir), "no directory");

    let mut project = Project {
        dir: dir.clone(),
        state: Default::default(),
    };
    project.state.compose_project = "demo-1a2b3c".to_string();
    project.save().unwrap();
    assert!(is_present("demo-1a2b3c", &dir));
    // `init --force` wrote a new deployment over it: the old name is gone.
    assert!(!is_present("demo-999999", &dir));
}
