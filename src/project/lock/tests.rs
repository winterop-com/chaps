use super::*;
use crate::project::ProjectState;

/// A command that read the state long ago and changes one thing does not
/// write back the rest of what it read over another command's save.
#[test]
fn update_saved_changes_the_state_on_disk_not_the_stale_copy() {
    let temp = tempfile::tempdir().unwrap();
    let mut early = Project {
        dir: temp.path().to_path_buf(),
        state: ProjectState::default(),
    };
    early.save().unwrap();

    // Meanwhile another command saves a change of its own.
    let mut other = Project::load(temp.path()).unwrap();
    other.state.api_port = 8799;
    other.save().unwrap();

    early
        .update_saved(|state| state.chap_image_tag = "v9.9.9".to_string())
        .unwrap();
    let saved = Project::load(temp.path()).unwrap();
    assert_eq!(saved.state.api_port, 8799, "the other save survives");
    assert_eq!(saved.state.chap_image_tag, "v9.9.9");
    assert_eq!(early.state.chap_image_tag, "v9.9.9", "the copy gets it too");
}
