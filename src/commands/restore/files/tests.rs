use super::*;

fn staged(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let varde = temp.path().join(VARDE_DIR);
    std::fs::create_dir_all(&varde).unwrap();
    let path = varde.join(PROJECT_FILE);
    std::fs::write(&path, body).unwrap();
    (temp, path)
}

const ARCHIVED: &str = "# Managed by varde; change it with the varde commands, not by hand.\n\
                        schema_version: 1\n\
                        compose_project: prod-1a2b3c\n\
                        api_port: 8700\n\
                        future_field: kept\n";

/// The arriving `project.yaml` names this deployment before any file lands,
/// so a restore that stops part-way leaves nothing pointed at the archive's
/// volumes; everything else in it is the archive's.
#[test]
fn the_staged_state_takes_this_deployments_name_before_anything_is_copied() {
    let (temp, path) = staged(ARCHIVED);
    stage_identity(temp.path(), "staging-4d5e6f", false).unwrap();
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.starts_with("# Managed by varde;"), "{body}");
    assert!(body.contains("compose_project: staging-4d5e6f"), "{body}");
    assert!(body.contains("future_field: kept"), "{body}");
    assert!(body.contains("api_port: 8700"), "{body}");
}

#[test]
fn adopting_the_identity_leaves_the_archived_name() {
    let (temp, path) = staged(ARCHIVED);
    stage_identity(temp.path(), "staging-4d5e6f", true).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), ARCHIVED);
}

#[test]
fn an_archive_without_a_name_keeps_this_deployments_own() {
    assert_eq!(identity("staging-4d5e6f", "", true), "staging-4d5e6f");
    assert_eq!(
        identity("staging-4d5e6f", "prod-1a2b3c", true),
        "prod-1a2b3c"
    );
    assert_eq!(
        identity("staging-4d5e6f", "prod-1a2b3c", false),
        "staging-4d5e6f"
    );
}
