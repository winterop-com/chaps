use super::*;

fn tracked() -> Vec<String> {
    THROWAWAYS.lock().unwrap().clone()
}

/// Each throwaway container gets a name of its own, and is tracked for a
/// second Ctrl-C only while its guard lives.
#[test]
fn a_throwaway_has_a_unique_name_that_is_tracked_while_it_runs() {
    let mut first = Throwaway::new("varde-test");
    let mut second = Throwaway::new("varde-test");
    assert_ne!(first.name(), second.name());
    assert!(
        first
            .name()
            .starts_with(&format!("varde-test-{}-", std::process::id())),
        "{}",
        first.name()
    );
    let (one, two) = (first.name().to_string(), second.name().to_string());
    assert!(tracked().contains(&one) && tracked().contains(&two));

    // A clean run: drop has nothing to remove, so no docker call is made.
    first.finished();
    second.finished();
    drop(first);
    assert!(!tracked().contains(&one));
    assert!(tracked().contains(&two));
    drop(second);
    assert!(!tracked().contains(&two));
}
