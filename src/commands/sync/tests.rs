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

fn text(report: &SyncReport) -> String {
    let mut lines = Report::default();
    say(report, Path::new("/p"), &mut lines);
    lines.text()
}

#[test]
fn a_sync_says_the_totals_and_lists_the_files_as_hints() {
    assert_eq!(
        text(&report(false)),
        "hint: wrote compose.chapkit-ewars-model.yml\n\
         hint: removed compose.auto-arima-chapkit.yml\n\
         hint: compose.marketplace.yml is unchanged\n\
         1 written, 1 unchanged, 1 removed\n"
    );
}

#[test]
fn check_mode_names_the_files_that_would_change() {
    let text = text(&report(true));
    assert!(text.starts_with("would write compose.chapkit-ewars-model.yml\n"));
    assert!(text.contains("would remove compose.auto-arima-chapkit.yml\n"));
    assert!(text.ends_with("1 to write, 1 unchanged, 1 to remove\n"));
}

#[test]
fn a_clean_check_says_in_sync() {
    let clean = SyncReport {
        unchanged: vec![PathBuf::from("/p/compose.marketplace.yml")],
        check: true,
        ..SyncReport::default()
    };
    assert!(text(&clean).ends_with("in sync: 0 to write, 1 unchanged, 0 to remove\n"));
}

#[test]
fn a_sync_warning_is_a_warning() {
    let warned = SyncReport {
        warnings: vec!["something to look at".to_string()],
        ..SyncReport::default()
    };
    assert!(text(&warned).starts_with("warning: something to look at\n"));
}
