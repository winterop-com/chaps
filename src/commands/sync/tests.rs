use super::*;
use std::path::PathBuf;

fn report(check: bool) -> SyncReport {
    SyncReport {
        written: vec![PathBuf::from("/p/compose.chapkit-ewars-model.yml")],
        unchanged: vec![PathBuf::from("/p/compose.marketplace.yml")],
        removed: vec![PathBuf::from("/p/compose.auto-arima-chapkit.yml")],
        drift: true,
        check,
        warnings: vec![],
    }
}

#[test]
fn human_lists_files_relative_to_the_project() {
    let text = human(&report(false), Path::new("/p"), &Out::default());
    assert_eq!(
        text,
        "written       compose.chapkit-ewars-model.yml\n\
             removed       compose.auto-arima-chapkit.yml\n\
             unchanged     compose.marketplace.yml\n\
             1 written, 1 unchanged, 1 removed\n"
    );
}

#[test]
fn check_mode_uses_the_conditional() {
    let text = human(&report(true), Path::new("/p"), &Out::default());
    assert!(text.starts_with("would write   compose.chapkit-ewars-model.yml\n"));
    assert!(text.contains("would remove  compose.auto-arima-chapkit.yml\n"));
    assert!(text.ends_with("1 to write, 1 unchanged, 1 to remove\n"));
}

#[test]
fn a_clean_check_says_in_sync() {
    let clean = SyncReport {
        unchanged: vec![PathBuf::from("/p/compose.marketplace.yml")],
        check: true,
        ..SyncReport::default()
    };
    let text = human(&clean, Path::new("/p"), &Out::default());
    assert!(text.ends_with("in sync: 0 to write, 1 unchanged, 0 to remove\n"));
}
