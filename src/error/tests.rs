use super::*;

#[test]
fn a_taken_port_says_who_has_it() {
    assert_eq!(
        ChapError::PortInUse {
            port: 5001,
            holder: PortHolder::ComposeFile,
        }
        .to_string(),
        "host port 5001 is already in use by another compose file; pick another with `--port <n>`, or `--port auto`"
    );
    assert_eq!(
        ChapError::PortInUse {
            port: 8000,
            holder: PortHolder::Host,
        }
        .to_string(),
        "host port 8000 is already in use by the host (something is listening); pick another with `--port <n>`, or `--port auto`"
    );
}

/// A directory under the `varde run` groups is a group that is not there,
/// with the `varde run` that makes it again; anywhere else is `varde init`.
#[test]
fn a_missing_group_is_not_told_to_run_init() {
    let groups = Path::new("/data/varde/run");
    let line = not_a_deployment(&groups.join("dengue"), groups);
    assert!(
        line.ends_with(
            "is not a varde deployment: the `varde run` group dengue does not exist, or \
             `varde stop --purge` removed it; `varde run <model> --group dengue` makes it again"
        ),
        "{line}"
    );
    let line = not_a_deployment(&groups.join("default").join("x"), groups);
    assert!(line.contains("`varde run <model>` makes it again"), "{line}");
    let line = not_a_deployment(Path::new("/srv/lab"), groups);
    assert!(line.ends_with("run `varde init` first"), "{line}");
    assert!(!not_a_deployment(groups, groups).contains("group"));
}
