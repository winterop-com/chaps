use super::*;
use std::path::Path;

fn external(url: &str) -> ExternalChapCore {
    ExternalChapCore {
        url: url.to_string(),
        models_host: "localhost".to_string(),
    }
}

#[test]
fn a_group_whose_chap_core_moves_says_how_the_running_models_follow() {
    let dir = Path::new("/home/me/.local/share/chaps/run/default");
    let note = changed_note(
        Some(&external("http://localhost:8000")),
        &external("http://localhost:8100"),
        dir,
    )
    .unwrap();
    assert!(
        note.contains("registered with http://localhost:8000"),
        "{note}"
    );
    assert!(note.contains("move to http://localhost:8100"), "{note}");
    assert!(note.contains("restart --all"), "{note}");

    // The same chap-core, or a first one: nothing to say.
    let same = external("http://localhost:8000");
    assert_eq!(changed_note(Some(&same), &same, dir), None);
    assert_eq!(changed_note(None, &same, dir), None);
}
