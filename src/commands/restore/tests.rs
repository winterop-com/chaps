use super::database::*;
use super::volumes::*;
use super::*;
use crate::backup::{
    DB_MEMBER, ENV_BACKUP_FILE, ManifestComponent, ManifestDatabase, ManifestModel,
    component_member, model_member,
};
use crate::project::ProjectState;
use std::path::PathBuf;

fn manifest() -> Manifest {
    Manifest {
        schema_version: crate::backup::SCHEMA_VERSION,
        created_by: "varde 0.1.0".into(),
        created_at: "2026-09-23T07:10:00Z".into(),
        project: "e2e".into(),
        chap_image_tag: "latest".into(),
        files: vec![
            ".env".into(),
            ".varde/models.yaml".into(),
            ".varde/project.yaml".into(),
            "ocs/climate-service.yaml".into(),
            "compose.yml".into(),
        ],
        database: Some(ManifestDatabase {
            path: DB_MEMBER.into(),
            user: "chap".into(),
            name: "chap_core".into(),
            server_version: Some("17.6".into()),
            size_bytes: 2048,
        }),
        models: vec![
            ManifestModel {
                id: "chapkit_ewars_model".into(),
                service_id: "chapkit-ewars-model".into(),
                version: "1.0.0".into(),
                image_tag: "sha-fa880a1".into(),
                host_port: Some(5001),
                data_dir: "/app/data".into(),
                user: "chapkit:chapkit".into(),
                volume: "ck_chapkit_ewars_model_data".into(),
                path: Some(model_member("chapkit-ewars-model")),
                size_bytes: 40960,
                skipped: None,
                failed: false,
                quiesce: Some("paused for 1.4 s".into()),
            },
            ManifestModel {
                id: "auto_arima_chapkit".into(),
                service_id: "auto-arima-chapkit".into(),
                version: "1.0.0".into(),
                image_tag: "sha-70c07a9".into(),
                host_port: Some(5002),
                data_dir: "/work/data".into(),
                user: "chapkit:chapkit".into(),
                volume: "ck_auto_arima_chapkit_data".into(),
                path: None,
                size_bytes: 0,
                skipped: Some("no volume yet".into()),
                failed: false,
                quiesce: None,
            },
        ],
        components: vec![
            ManifestComponent {
                name: "ocs".into(),
                service: "ocs".into(),
                volume: "ocs_data".into(),
                data_dir: "/app/data".into(),
                path: Some(component_member("ocs")),
                size_bytes: 4096,
                skipped: None,
                failed: false,
                quiesce: Some("paused for 0.3 s".into()),
            },
            ManifestComponent {
                name: "s3".into(),
                service: "s3".into(),
                volume: "s3_data".into(),
                data_dir: "/data".into(),
                path: None,
                size_bytes: 0,
                skipped: Some("no volume yet".into()),
                failed: false,
                quiesce: None,
            },
        ],
    }
}

fn members() -> BTreeSet<String> {
    [
        MANIFEST_MEMBER.to_string(),
        "files/.env".to_string(),
        "files/.varde/models.yaml".to_string(),
        "files/.varde/project.yaml".to_string(),
        "files/ocs/climate-service.yaml".to_string(),
        "files/compose.yml".to_string(),
        DB_MEMBER.to_string(),
        model_member("chapkit-ewars-model"),
        component_member("ocs"),
    ]
    .into_iter()
    .collect()
}

fn project() -> Project {
    Project {
        dir: PathBuf::from("/srv/e2e"),
        state: ProjectState {
            compose_project: "e2e-ab12cd".to_string(),
            ..ProjectState::default()
        },
    }
}

fn args(flags: &[&str]) -> RestoreArgs {
    use clap::Parser;
    let mut argv = vec!["chap", "backup", "restore", "/backups/x.tar.gz"];
    argv.extend_from_slice(flags);
    let cli = crate::cli::Cli::try_parse_from(argv).unwrap();
    let crate::cli::Command::Backup(b) = cli.command else {
        panic!("expected backup");
    };
    let crate::cli::BackupSub::Restore(args) = b.command else {
        panic!("expected restore");
    };
    args
}

fn running(services: &[&str]) -> BTreeSet<String> {
    services.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_full_restore_of_a_running_stack_plans_everything() {
    let up = running(&[
        "chap",
        "worker",
        "postgres",
        "chapkit-ewars-model",
        "ocs",
        "redis",
    ]);
    let plan = plan_with(&up, &[]);
    assert_eq!(plan.files.len(), 5);
    assert!(plan.database);
    assert_eq!(
        plan.models
            .iter()
            .map(|m| &m.service_id)
            .collect::<Vec<_>>(),
        vec!["chapkit-ewars-model"],
        "the model with no data in the archive is not planned"
    );
    assert_eq!(
        plan.components.iter().map(|c| &c.name).collect::<Vec<_>>(),
        vec!["ocs"],
        "the component with no data in the archive is not planned"
    );
    // postgres and redis are left alone: the restore needs postgres, and
    // redis holds nothing this touches. ocs holds its own volume, so it
    // goes down with the models.
    assert_eq!(
        plan.stop,
        vec!["chap", "worker", "chapkit-ewars-model", "ocs"]
    );
    assert!(plan.start);
}

#[test]
fn a_stopped_stack_is_not_stopped_again() {
    let plan = plan_with(&BTreeSet::new(), &[]);
    assert!(plan.stop.is_empty());
}

#[test]
fn files_only_touches_no_docker_at_all() {
    let up = running(&["chap", "worker", "postgres"]);
    let plan = plan_with(&up, &["--files-only"]);
    assert_eq!(plan.files.len(), 5);
    assert!(!plan.database);
    assert!(plan.models.is_empty());
    assert!(plan.components.is_empty());
    assert!(plan.stop.is_empty());
    assert!(!plan.start, "there is nothing to start");
}

#[test]
fn db_only_and_the_no_flags_narrow_the_plan() {
    let up = running(&["chap", "worker", "chapkit-ewars-model", "ocs"]);
    let plan = plan_with(&up, &["--db-only"]);
    assert!(plan.files.is_empty() && plan.database);
    assert!(plan.models.is_empty() && plan.components.is_empty());
    assert_eq!(
        plan.stop,
        vec!["chap", "worker"],
        "no model and no component is disturbed"
    );

    let plan = plan_with(&up, &["--no-models"]);
    assert_eq!(plan.files.len(), 5);
    assert!(plan.database && plan.models.is_empty());
    assert_eq!(
        plan.components.len(),
        1,
        "--no-models is about the models alone"
    );
    assert_eq!(plan.stop, vec!["chap", "worker", "ocs"]);

    let plan = plan_with(&up, &["--no-components"]);
    assert!(plan.components.is_empty() && plan.models.len() == 1);
    assert_eq!(plan.stop, vec!["chap", "worker", "chapkit-ewars-model"]);

    let plan = plan_with(&up, &["--no-start"]);
    assert!(!plan.start);
}

fn plan_with(up: &BTreeSet<String>, flags: &[&str]) -> RestorePlan {
    plan_for(up, flags, Some("e2e-ab12cd"))
}

fn plan_for(up: &BTreeSet<String>, flags: &[&str], archived: Option<&str>) -> RestorePlan {
    plan(
        &project(),
        Path::new("/backups/x.tar.gz"),
        manifest(),
        &members(),
        up,
        archived.map(str::to_string),
        &args(flags),
    )
}

#[test]
fn a_file_the_manifest_lists_but_the_archive_lacks_is_not_planned() {
    let mut members = members();
    members.remove("files/compose.yml");
    let plan = plan(
        &project(),
        Path::new("/backups/x.tar.gz"),
        manifest(),
        &members,
        &BTreeSet::new(),
        None,
        &args(&[]),
    );
    assert_eq!(
        plan.files,
        vec![
            ".env",
            ".varde/models.yaml",
            ".varde/project.yaml",
            "ocs/climate-service.yaml"
        ]
    );
}

#[test]
fn an_archive_with_nothing_in_it_plans_nothing() {
    let mut manifest = manifest();
    manifest.files.clear();
    manifest.database = None;
    manifest.models.clear();
    manifest.components.clear();
    let plan = plan(
        &project(),
        Path::new("/backups/x.tar.gz"),
        manifest,
        &BTreeSet::new(),
        &BTreeSet::new(),
        None,
        &args(&[]),
    );
    assert!(plan.is_empty());
}

#[test]
fn the_plan_keeps_this_deployments_identity_unless_it_is_told_not_to() {
    let up = BTreeSet::new();

    // An archive from another deployment: this one keeps its own name, so
    // its containers and volumes are the ones refilled.
    let plan = plan_for(&up, &[], Some("chapx-9f01bc"));
    assert_eq!(plan.compose_project, "e2e-ab12cd");
    assert_eq!(
        plan.archived_compose_project.as_deref(),
        Some("chapx-9f01bc")
    );
    assert!(!plan.adopt_identity);
    assert!(backup::plan_text(&plan).contains("the archive's own (chapx-9f01bc)"));

    // --adopt-identity is the takeover.
    let plan = plan_for(&up, &["--adopt-identity"], Some("chapx-9f01bc"));
    assert_eq!(plan.compose_project, "chapx-9f01bc");
    assert!(plan.adopt_identity);

    // Nothing to adopt when the archive records no name, or when the file
    // that holds it is not among the ones being written.
    let plan = plan_for(&up, &["--adopt-identity"], None);
    assert_eq!(plan.compose_project, "e2e-ab12cd");
    assert!(plan.archived_compose_project.is_none());

    let plan = plan_for(&up, &["--db-only"], Some("chapx-9f01bc"));
    assert!(
        plan.archived_compose_project.is_none(),
        "--db-only writes no project.yaml, so no identity is at stake"
    );
}

fn report(started: bool, warnings: Vec<&str>) -> RestoreReport {
    RestoreReport {
        plan: plan_with(&running(&["chap", "worker"]), &[]),
        files: vec![".env".into(), ".varde/models.yaml".into()],
        removed_state: Vec::new(),
        env_backup: Some(ENV_BACKUP_FILE.to_string()),
        kept_credentials: Vec::new(),
        database: true,
        database_warnings: warnings.into_iter().map(str::to_string).collect(),
        models: vec!["chapkit-ewars-model".into()],
        components: vec!["ocs".into()],
        stopped: vec!["chap".into(), "worker".into()],
        started,
    }
}

fn rendered(report: &RestoreReport) -> String {
    let mut lines = output::Report::default();
    say(report, &mut lines);
    lines.text()
}

#[test]
fn the_summary_is_one_line_with_the_details_as_hints() {
    let text = rendered(&report(true, vec![]));
    assert!(
        text.starts_with(
            "restored 2 files, the chap_core database, the data of chapkit-ewars-model, ocs\n"
        ),
        "{text}"
    );
    assert!(text.contains("hint: stopped chap, worker first\n"));
    assert!(text.contains("hint: files: .env, .varde/models.yaml\n"));
    assert!(text.contains("hint: the previous .env is in .env.before-restore\n"));
    assert!(text.contains("\nthe deployment is starting\n"));
    // This archive came from this deployment, so there is nothing to say
    // about whose identity it kept.
    assert!(!text.contains("compose project"), "{text}");
}

#[test]
fn the_summary_says_whose_identity_the_deployment_kept() {
    let mut report = report(true, vec![]);
    report.plan = plan_for(&running(&[]), &[], Some("chapx-9f01bc"));
    let text = rendered(&report);
    assert!(
        text.contains("hint: e2e-ab12cd is kept; the archive's own (chapx-9f01bc) is not adopted"),
        "{text}"
    );

    let mut report = self::report(true, vec![]);
    report.plan = plan_for(&running(&[]), &["--adopt-identity"], Some("chapx-9f01bc"));
    let text = rendered(&report);
    assert!(
        text.contains("\ncompose project chapx-9f01bc, taken over from the archive"),
        "{text}"
    );
}

#[test]
fn pg_restore_warnings_are_counted_not_hidden() {
    let text = rendered(&report(
        false,
        vec!["warning: errors ignored on restore: 2"],
    ));
    assert!(text.contains("hint: pg_restore gave 1 warning(s), shown above\n"));
    assert!(text.ends_with("\nrun `varde up` to apply\n"), "{text}");
}

#[test]
fn a_run_that_restored_nothing_says_so() {
    let mut report = report(true, vec![]);
    report.files.clear();
    report.env_backup = None;
    report.database = false;
    report.models.clear();
    report.components.clear();
    report.stopped.clear();
    let text = rendered(&report);
    assert!(text.starts_with("restored nothing\n"));
    assert!(!text.contains("stopped"));
}

#[test]
fn a_failed_pg_restore_is_quoted_by_the_line_that_says_why() {
    // The reason is the last error, not the first line of the stderr.
    let stderr = "pg_restore: connecting to database for restore\n\
                      pg_restore: error: connection to server at \"postgres\" failed: \
                      FATAL:  role \"nosuchrole\" does not exist\n";
    assert!(failure_reason(stderr).starts_with("connection to server"));
    // Rows that never loaded: that error, with the statement, ahead of
    // any harmless drop printed after it.
    let lost = "pg_restore: error: could not execute query: ERROR:  relation \"public.jobs\" \
                    does not exist\n\
                    Command was: COPY public.jobs (id, name) FROM stdin;\n\
                    pg_restore: error: could not execute query: ERROR:  schema \"x\" does not exist\n\
                    Command was: DROP SCHEMA IF EXISTS x;\n\
                    pg_restore: warning: errors ignored on restore: 2\n";
    assert_eq!(
        failure_reason(lost),
        "could not execute query: ERROR:  relation \"public.jobs\" does not exist \
             (while running `COPY public.jobs (id, name) FROM stdin;`)"
    );
    // Nothing that looks like an error: the last thing it printed.
    assert_eq!(failure_reason("out of disk\n\n"), "out of disk");
    assert_eq!(failure_reason(""), "no output");
}

#[test]
fn the_database_is_reached_through_exec_without_a_terminal() {
    assert_eq!(
        exec_args("postgres", &["psql", "-U", "chap", "-tAc", "select 1"]),
        vec![
            "exec", "-T", "postgres", "psql", "-U", "chap", "-tAc", "select 1"
        ]
    );
}

#[test]
fn the_init_container_is_always_run_without_a_terminal_or_dependencies() {
    assert_eq!(
        run_args(
            "chapkit-ewars-model-init",
            &["sh".to_string(), "-c".to_string(), "tar xf -".to_string()]
        ),
        vec![
            "run",
            "--rm",
            "--no-deps",
            "-T",
            "chapkit-ewars-model-init",
            "sh",
            "-c",
            "tar xf -"
        ]
    );
}
