use super::*;
use crate::commands::enable::DisableReport;

fn disable(purged: &[&str]) -> DisableReport {
    DisableReport {
        apply: Default::default(),
        stopped: None,
        purged: purged.iter().map(|v| v.to_string()).collect(),
        kept_volumes: Vec::new(),
    }
}

fn stopped(id: &str, group: Option<&str>, notes: &[&str], purged: &[&str]) -> StoppedModel {
    StoppedModel {
        group: group.map(str::to_string),
        id: id.to_string(),
        report: disable(purged),
        notes: notes.iter().map(|n| n.to_string()).collect(),
    }
}

fn said(report: &StopReport) -> String {
    let mut lines = Report::default();
    say(report, &mut lines);
    lines.text()
}

/// One stopped model is one line; what was kept and how to start it again
/// are hints.
#[test]
fn a_stop_is_one_line_and_the_rest_is_hints() {
    let kept =
        crate::commands::docker::kept_volume_line("g_ck_m_data", Some("varde models disable m"));
    let report = StopReport {
        stopped: vec![stopped(
            "m",
            Some("default"),
            &[
                "stopped and removed the m container; the host port it published is free again",
                &kept,
            ],
            &[],
        )],
        removed: Vec::new(),
        removed_volumes: Vec::new(),
    };
    assert_eq!(
        said(&report),
        format!(
            "stopped m\n\
             hint: stopped and removed the m container; the host port it published is free again\n\
             hint: {kept}\n\
             hint: `varde run m` starts it again\n"
        )
    );
}

/// What `--purge` removed is info, and a group other than the default is
/// named.
#[test]
fn a_purge_says_what_went() {
    let report = StopReport {
        stopped: vec![stopped("m", Some("trial"), &["removed volume v1"], &["v1"])],
        removed: vec!["trial".to_string()],
        removed_volumes: vec!["v2".to_string()],
    };
    assert_eq!(
        said(&report),
        "stopped m (group trial)\n\
         removed volume v1\n\
         removed group trial\n\
         hint: removed volume v2\n\
         hint: `varde run m --group trial` starts it again\n"
    );
}

/// A container or a volume that did not go needs attention.
#[test]
fn what_did_not_work_is_a_warning() {
    let report = StopReport {
        stopped: vec![stopped(
            "m",
            None,
            &[
                "the m container could not be stopped; `varde up` removes it as an orphan",
                "volume v could not be removed: in use",
                crate::commands::docker::UNNAMEABLE_VOLUME,
            ],
            &[],
        )],
        removed: Vec::new(),
        removed_volumes: Vec::new(),
    };
    let text = said(&report);
    assert_eq!(text.matches("warning: ").count(), 3, "{text}");
}

#[test]
fn nothing_to_stop_says_so() {
    let report = StopReport {
        stopped: Vec::new(),
        removed: Vec::new(),
        removed_volumes: Vec::new(),
    };
    assert_eq!(
        said(&report),
        "nothing was running to stop\nhint: `varde ps` lists what runs\n"
    );
}
