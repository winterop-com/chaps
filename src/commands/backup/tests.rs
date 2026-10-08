use super::capture::*;
use super::quiesce::*;
use super::*;
use crate::backup::{DB_MEMBER, ManifestComponent, ManifestDatabase, ManifestModel, model_member};
use std::time::Duration;

fn model(service_id: &str, skipped: Option<&str>) -> ManifestModel {
    ManifestModel {
        id: service_id.replace('-', "_"),
        service_id: service_id.to_string(),
        version: "1.0.0".into(),
        image_tag: "sha-fa880a1".into(),
        host_port: Some(5001),
        data_dir: "/app/data".into(),
        user: "chapkit:chapkit".into(),
        volume: format!("ck_{}_data", service_id.replace('-', "_")),
        path: skipped.is_none().then(|| model_member(service_id)),
        size_bytes: if skipped.is_none() { 40960 } else { 0 },
        skipped: skipped.map(str::to_string),
        failed: false,
        quiesce: skipped.is_none().then(|| "paused for 1.4 s".to_string()),
    }
}

fn component(name: &str, skipped: Option<&str>) -> ManifestComponent {
    ManifestComponent {
        name: name.to_string(),
        service: name.to_string(),
        volume: format!("{name}_data"),
        data_dir: "/app/data".into(),
        path: skipped.is_none().then(|| backup::component_member(name)),
        size_bytes: if skipped.is_none() { 4096 } else { 0 },
        skipped: skipped.map(str::to_string),
        failed: false,
        quiesce: None,
    }
}

fn report_with(
    database: bool,
    models: Vec<ManifestModel>,
    components: Vec<ManifestComponent>,
) -> BackupReport {
    BackupReport {
        path: PathBuf::from("/backups/varde-backup-e2e-20260923-071000.tar.gz"),
        size_bytes: 5 * 1024 * 1024,
        manifest: Manifest {
            schema_version: crate::backup::SCHEMA_VERSION,
            created_by: "varde 0.1.0".into(),
            created_at: "2026-09-23T07:10:00Z".into(),
            project: "e2e".into(),
            chap_image_tag: "latest".into(),
            files: vec![".env".into(), "compose.yml".into()],
            database: database.then(|| ManifestDatabase {
                path: DB_MEMBER.into(),
                user: "chap".into(),
                name: "chap_core".into(),
                server_version: Some("17.6".into()),
                size_bytes: 2048,
            }),
            models,
            components,
        },
        no_chap_core: false,
    }
}

fn rendered(report: &BackupReport) -> String {
    let mut lines = output::Report::default();
    say(report, &mut lines);
    lines.text()
}

#[test]
fn the_output_is_one_line_and_the_parts_are_hints() {
    let text = rendered(&report_with(
        true,
        vec![model("chapkit-ewars-model", None)],
        vec![component("ocs", None)],
    ));
    assert!(text.starts_with(
        "wrote /backups/varde-backup-e2e-20260923-071000.tar.gz \
         (5.0 MB gzipped, 46.0 KB of data)\n"
    ));
    assert!(text.contains("hint: files: .env, compose.yml\n"));
    assert!(text.contains("hint: database: chap_core as chap (2.0 KB), PostgreSQL 17.6\n"));
    // A running service was held still for the read, and says for how long.
    assert!(
        text.contains("hint: model chapkit-ewars-model: /app/data (40.0 KB, paused for 1.4 s)\n")
    );
    assert!(text.contains("hint: component ocs: /app/data (4.0 KB)\n"));
    assert!(!text.contains("warning:"));
    // And it ends on the command that reads the archive back.
    assert!(text.ends_with(
        "hint: `varde backup restore /backups/varde-backup-e2e-20260923-071000.tar.gz` \
         restores it\n"
    ));
    assert_eq!(text.lines().filter(|l| !l.starts_with("hint:")).count(), 1);
}

#[test]
fn what_was_left_out_is_a_warning_unless_a_flag_asked_for_it() {
    let text = rendered(&report_with(
        false,
        vec![model("auto-arima-chapkit", Some("no volume yet"))],
        vec![component("s3", Some("--no-components"))],
    ));
    assert!(text.contains("hint: database: not included (--no-db)\n"));
    assert!(text.contains("warning: auto-arima-chapkit: no volume yet\n"));
    assert!(text.contains("hint: s3: not included (--no-components)\n"));
    assert!(!text.contains("model "));
    assert!(!text.contains("component "));
}

#[test]
fn the_two_dhis2_volumes_left_out_by_a_flag_are_one_line() {
    let mut db = component("dhis2", Some("--no-components"));
    db.service = "dhis2-db".into();
    db.volume = "dhis2_db".into();
    let text = rendered(&report_with(
        false,
        vec![],
        vec![component("dhis2", Some("--no-components")), db],
    ));
    assert_eq!(
        text.matches("hint: dhis2: not included (--no-components)\n")
            .count(),
        1,
        "{text}"
    );
}

#[test]
fn a_service_that_was_not_running_is_not_reported_as_paused() {
    assert_eq!(size_note(4096, None), "(4.0 KB)");
    assert_eq!(
        size_note(4096, Some("stopped for 6.0 s")),
        "(4.0 KB, stopped for 6.0 s)"
    );
    assert_eq!(
        quiesce_note("paused", Duration::from_millis(1440)),
        "paused for 1.4 s"
    );
    assert_eq!(
        quiesce_note("stopped", Duration::from_millis(40)),
        "stopped for 0.0 s"
    );
}

#[test]
fn a_failed_pack_leaves_the_archive_that_is_already_there_alone() {
    let dir = tempfile::tempdir().unwrap();
    let stage = dir.path().join("stage");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("manifest.yaml"), "schema_version: 1\n").unwrap();

    let out = dir.path().join("nightly.tar.gz");
    std::fs::write(&out, b"last night's backup").unwrap();

    // tar cannot pack a member that is not in the stage, so this fails
    // after the temporary file has been created.
    let err = write_archive(
        &out,
        &stage,
        &["manifest.yaml".to_string(), "db".to_string()],
    )
    .expect_err("packing a missing member fails");
    assert!(err.to_string().contains("tar failed"), "{err}");
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"last night's backup",
        "the archive that was already there is untouched"
    );
    assert!(
        !backup::temp_archive_path(&out).exists(),
        "and the half-written one is gone"
    );

    // The same call with a member that is there renames into place.
    write_archive(&out, &stage, &["manifest.yaml".to_string()]).unwrap();
    assert_eq!(
        backup::tar_read_member(&out, "manifest.yaml").unwrap(),
        "schema_version: 1\n"
    );
    assert!(!backup::temp_archive_path(&out).exists());
}

#[test]
fn the_compose_argument_lists_never_ask_for_a_terminal() {
    assert_eq!(
        compose_exec("postgres", &["pg_dump", "-U", "chap", "-Fc", "chap_core"]),
        vec![
            "exec",
            "-T",
            "postgres",
            "pg_dump",
            "-U",
            "chap",
            "-Fc",
            "chap_core"
        ]
    );
    assert_eq!(
        compose_run(
            "chapkit-ewars-model-init",
            &["tar", "cf", "-", "-C", "/app/data", "."]
        ),
        vec![
            "run",
            "--rm",
            "--no-deps",
            "-T",
            "chapkit-ewars-model-init",
            "tar",
            "cf",
            "-",
            "-C",
            "/app/data",
            "."
        ]
    );
}

#[test]
fn the_project_name_falls_back_to_something_printable() {
    assert_eq!(project_name(Path::new("/srv/e2e")), "e2e");
    assert_eq!(project_name(Path::new("/")), "varde");
}

/// A read that failed makes the run fail; data there was no reason to read -
/// a model never started - does not.
#[test]
fn only_a_failed_read_fails_the_backup() {
    let mut broken = model(
        "auto-arima-chapkit",
        Some("reading /work/data failed (exit 1): x"),
    );
    broken.failed = true;
    let mut ocs = component("ocs", Some("reading ocs_data failed (exit 1): x"));
    ocs.failed = true;
    let report = report_with(
        true,
        vec![
            model("chapkit-ewars-model", None),
            model("never-started", Some("no volume yet")),
            broken,
        ],
        vec![ocs],
    );
    assert_eq!(
        failed_reads(&report.manifest),
        vec!["auto-arima-chapkit", "ocs_data"]
    );
    let fine = report_with(
        true,
        vec![model("never-started", Some("no volume yet"))],
        vec![],
    );
    assert!(failed_reads(&fine.manifest).is_empty());
}

/// A large volume of a running service is said before the pause, because the
/// service does not answer until the copy is done.
#[test]
fn a_long_pause_is_said_before_it_starts() {
    use super::capture::{LONG_PAUSE_BYTES, Omit, pause_warning_for};
    assert_eq!(
        pause_warning_for(
            "dhis2",
            "x_dhis2_home",
            LONG_PAUSE_BYTES - 1,
            Omit::Components
        ),
        None
    );
    let warning = pause_warning_for(
        "dhis2",
        "x_dhis2_home",
        40 * LONG_PAUSE_BYTES,
        Omit::Components,
    )
    .expect("a large volume");
    assert!(
        warning.starts_with("dhis2 is paused while varde copies 40.0 GB of `x_dhis2_home`"),
        "{warning}"
    );
    assert!(warning.contains("`--no-components`"), "{warning}");

    // A model is paused the same way, and is left out by its own flag.
    let warning = pause_warning_for(
        "chapkit-ewars-model",
        "x_ck_chapkit_ewars_model_data",
        LONG_PAUSE_BYTES,
        Omit::Models,
    )
    .expect("a large volume");
    assert!(
        warning.starts_with("chapkit-ewars-model is paused while varde copies 1.0 GB"),
        "{warning}"
    );
    assert!(
        warning.ends_with("use `--no-models` to omit the data of every model"),
        "{warning}"
    );
}
