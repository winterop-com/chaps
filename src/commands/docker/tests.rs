use super::*;
use crate::cli::{ConfigArgs, DownArgs, ExecArgs, LogsArgs, PsArgs, PullArgs, RunArgs, UpArgs};

/// A `docker compose ps` that ran and was refused.
fn refused_ps() -> docker::QueryFailure {
    docker::QueryFailure {
        query: "ps -a --format json".to_string(),
        code: Some(1),
        detail: "error during connect: the daemon is not running".to_string(),
    }
}

#[test]
fn a_wrapper_that_could_not_ask_docker_first_says_what_docker_said() {
    let err = docker_failed(1, Some(refused_ps()));
    assert_eq!(
        err.to_string(),
        "docker could not be asked about this project: `docker compose ps -a --format json` \
             exited with status 1: error during connect: the daemon is not running"
    );
    // The status stays in the chain: `main` reads the code off it.
    assert!(matches!(
        err.downcast_ref::<ChapError>(),
        Some(ChapError::DockerFailed(1))
    ));
    assert_eq!(
        err.chain().nth(1).map(|c| c.to_string()),
        Some("docker compose exited with status 1".to_string())
    );
}

/// The three lines a `--purge` can print, and the one a disable without it
/// prints instead.
#[test]
fn a_volume_is_reported_by_name_whatever_became_of_it() {
    let volume = "mychap-1ab2c3_ck_chapkit_ewars_model_data";
    assert_eq!(
        removal_line(volume, &docker::Removal::Removed),
        format!("removed volume {volume}")
    );
    assert_eq!(
        removal_line(volume, &docker::Removal::NotFound),
        format!("volume {volume} not found")
    );
    assert_eq!(
        removal_line(
            volume,
            &docker::Removal::Refused("volume is in use".to_string())
        ),
        format!("volume {volume} could not be removed: volume is in use")
    );

    // The kept line names both ways out, because the second one still
    // works once the deployment directory is gone.
    assert_eq!(
        kept_volume_line(volume, Some("chaps models disable chapkit_ewars_model")),
        format!(
            "kept volume {volume}; remove it with \
                 `chaps models disable chapkit_ewars_model --purge` or \
                 `docker volume rm {volume}`"
        )
    );

    // After `chaps models remove` no chaps command names the volume any
    // more, so only the docker command is offered.
    assert_eq!(
        kept_volume_line(volume, None),
        format!("kept volume {volume}; remove it with `docker volume rm {volume}`")
    );
}

#[test]
fn a_wrapper_docker_answered_for_ends_on_the_status_alone() {
    let err = docker_failed(137, None);
    assert_eq!(err.to_string(), "docker compose exited with status 137");
    assert_eq!(err.chain().count(), 1);
}

/// A plain terminal session: no `--json`, stdin is a TTY.
const TERM: Shell = Shell {
    json: false,
    tty: true,
};
/// A script or a pipe: no terminal on stdin.
const SCRIPT: Shell = Shell {
    json: false,
    tty: false,
};
/// `--json` asked for, from a terminal.
const JSON: Shell = Shell {
    json: true,
    tty: true,
};

fn up(attach: bool, extra: &[&str]) -> DockerCmd {
    DockerCmd::Up(UpArgs {
        attach,
        pull: false,
        no_preflight: false,
        replace: false,
        extra: extra.iter().map(|s| s.to_string()).collect(),
    })
}

/// The `DockerCmd` behind `chaps down`, with its two flags.
fn down(volumes: bool, extra: &[&str]) -> DockerCmd {
    DockerCmd::Down(DownArgs {
        volumes,
        yes: true,
        extra: extra.iter().map(|s| s.to_string()).collect(),
    })
}

fn exec(service: &str, cmd: &[&str]) -> DockerCmd {
    DockerCmd::Exec(ExecArgs {
        service: service.to_string(),
        cmd: cmd.iter().map(|s| s.to_string()).collect(),
    })
}

#[test]
fn up_detaches_unless_attach_is_asked_for() {
    assert_eq!(
        args_for(&up(false, &[]), TERM),
        vec!["up", "-d", "--remove-orphans"]
    );
    assert_eq!(
        args_for(&up(true, &[]), TERM),
        vec!["up", "--remove-orphans"]
    );
}

/// `chaps` renders every file in the `-f` list, so a container whose
/// service is in none of them belongs to something that was disabled.
/// Leaving it running would keep its host port published after a
/// `models disable` said it was gone.
#[test]
fn up_and_restart_take_the_orphans_of_a_disabled_service_with_them() {
    for cmd in [up(false, &[]), up(true, &[]), restart(false, &[])] {
        assert!(
            args_for(&cmd, TERM).iter().any(|a| a == "--remove-orphans"),
            "{cmd:?}"
        );
    }
    // Once, even when the caller asked for it as well: `chaps up
    // --remove-orphans` is a thing people type.
    let asked = args_for(&up(false, &["--remove-orphans"]), TERM);
    assert_eq!(asked.iter().filter(|a| *a == "--remove-orphans").count(), 1);
    assert_eq!(asked, vec!["up", "-d", "--remove-orphans"]);
    // Every other wrapper is docker's own command, untouched.
    for cmd in [down(false, &[]), exec("chap", &[])] {
        assert!(!args_for(&cmd, TERM).iter().any(|a| a == "--remove-orphans"));
    }
}

#[test]
fn up_pull_asks_compose_to_pull_always() {
    let cmd = DockerCmd::Up(UpArgs {
        attach: false,
        pull: true,
        no_preflight: false,
        replace: false,
        extra: vec!["chap".to_string()],
    });
    assert_eq!(
        args_for(&cmd, TERM),
        vec!["up", "-d", "--remove-orphans", "--pull", "always", "chap"]
    );
    let cmd = DockerCmd::Up(UpArgs {
        attach: true,
        pull: true,
        no_preflight: true,
        replace: false,
        extra: vec![],
    });
    assert_eq!(
        args_for(&cmd, TERM),
        vec!["up", "--remove-orphans", "--pull", "always"]
    );
}

#[test]
fn up_passes_extra_arguments_through_after_the_flags() {
    assert_eq!(
        args_for(&up(false, &["--build", "chap"]), TERM),
        vec!["up", "-d", "--remove-orphans", "--build", "chap"]
    );
    assert_eq!(
        args_for(&up(true, &["chap"]), TERM),
        vec!["up", "--remove-orphans", "chap"],
        "--attach must not inject -d before the service name"
    );
}

#[test]
fn down_and_pull_are_bare_commands() {
    assert_eq!(args_for(&down(false, &[]), TERM), vec!["down"]);
    assert_eq!(
        args_for(&down(false, &["--timeout", "30"]), TERM),
        vec!["down", "--timeout", "30"]
    );
    assert_eq!(
        args_for(&DockerCmd::Pull(PullArgs {}), JSON),
        vec!["pull"],
        "--json does not change pull"
    );
}

/// The `RestartArgs` a `chaps restart` would have parsed.
fn restart(all: bool, services: &[&str]) -> DockerCmd {
    DockerCmd::Restart(crate::cli::RestartArgs {
        all,
        services: services.iter().map(|s| s.to_string()).collect(),
    })
}

#[test]
fn restart_is_an_up_that_recreates_only_what_moved() {
    // The whole project: compose decides, service by service.
    assert_eq!(
        args_for(&restart(false, &[]), TERM),
        vec!["up", "-d", "--remove-orphans"]
    );
    // Named services are recreated on their own.
    assert_eq!(
        args_for(&restart(false, &["chap", "worker"]), TERM),
        vec![
            "up",
            "-d",
            "--remove-orphans",
            "--no-deps",
            "chap",
            "worker"
        ]
    );
    // --all is what turns a restart into an unconditional one.
    assert_eq!(
        args_for(&restart(true, &[]), TERM),
        vec!["up", "-d", "--remove-orphans", "--force-recreate"]
    );
    assert_eq!(
        args_for(&restart(true, &["chapkit-ewars-model"]), TERM),
        vec![
            "up",
            "-d",
            "--remove-orphans",
            "--force-recreate",
            "--no-deps",
            "chapkit-ewars-model"
        ]
    );
    // Nothing about it depends on --json.
    assert_eq!(
        args_for(&restart(false, &[]), JSON),
        vec!["up", "-d", "--remove-orphans"]
    );
}

#[test]
fn restart_says_which_services_it_recreated() {
    let before = vec![
        container("chap", "aaa", "t1"),
        container("worker", "bbb", "t1"),
        container("postgres", "ccc", "t1"),
    ];
    let after = vec![
        container("chap", "ddd", "t2"),
        container("worker", "eee", "t2"),
        container("postgres", "ccc", "t1"),
    ];
    assert_eq!(
        restart_summary(&Out::default(), &before, &after, &[]),
        "recreated: chap, worker; unchanged: postgres\n\
         run `chaps status` to check that everything answers"
    );
    // Everything moved: there is no second half to print.
    assert_eq!(
        restart_summary(
            &Out::default(),
            &before,
            &[container("chap", "ddd", "t2")],
            &[]
        ),
        "recreated: chap\nrun `chaps status` to check that everything answers"
    );
    // Nothing moved, which is an answer and not a failure, and the way to
    // force it names what was asked for.
    assert_eq!(
        restart_summary(&Out::default(), &before, &before, &["ocs".to_string()]),
        "nothing needed a restart: every container matches its files; `chaps restart --all \
         ocs` recreates it anyway"
    );
    assert!(
        restart_summary(&Out::default(), &before, &before, &[])
            .ends_with("`chaps restart --all` recreates every one anyway")
    );
}

#[test]
fn ps_asks_docker_for_json_under_the_global_flag() {
    let cmd = DockerCmd::Ps(PsArgs { extra: vec![] });
    assert_eq!(args_for(&cmd, TERM), vec!["ps"]);
    assert_eq!(args_for(&cmd, JSON), vec!["ps", "--format", "json"]);

    let cmd = DockerCmd::Ps(PsArgs {
        extra: vec!["-a".to_string()],
    });
    assert_eq!(args_for(&cmd, JSON), vec!["ps", "--format", "json", "-a"]);
}

#[test]
fn logs_maps_follow_and_service_names() {
    let cmd = DockerCmd::Logs(LogsArgs {
        follow: true,
        services: vec!["chap".to_string(), "chapkit-ewars-model".to_string()],
    });
    assert_eq!(
        args_for(&cmd, TERM),
        vec!["logs", "-f", "chap", "chapkit-ewars-model"]
    );

    let cmd = DockerCmd::Logs(LogsArgs {
        follow: false,
        services: vec![],
    });
    assert_eq!(args_for(&cmd, TERM), vec!["logs"]);
}

#[test]
fn exec_falls_back_to_a_shell() {
    assert_eq!(
        args_for(&exec("chap", &[]), TERM),
        vec!["exec", "chap", "sh"]
    );
    assert_eq!(
        args_for(&exec("chap", &["env"]), TERM),
        vec!["exec", "chap", "env"]
    );
}

#[test]
fn exec_asks_for_no_tty_when_there_is_none() {
    assert_eq!(
        args_for(&exec("chap", &["echo", "hello"]), SCRIPT),
        vec!["exec", "-T", "chap", "echo", "hello"],
        "-T goes before the service name, where compose expects its flags"
    );
    assert_eq!(
        args_for(&exec("chap", &[]), SCRIPT),
        vec!["exec", "-T", "chap", "sh"]
    );
}

#[test]
fn exec_hands_the_container_its_own_flags() {
    assert_eq!(
        args_for(&exec("chap", &["ls", "-la", "/app"]), TERM),
        vec!["exec", "chap", "ls", "-la", "/app"]
    );
}

#[test]
fn run_is_passed_through_verbatim() {
    let cmd = DockerCmd::Run(RunArgs {
        args: vec![
            "config".to_string(),
            "--services".to_string(),
            "--quiet".to_string(),
        ],
    });
    assert_eq!(
        args_for(&cmd, JSON),
        vec!["config", "--services", "--quiet"]
    );
    assert_eq!(
        args_for(&cmd, SCRIPT),
        vec!["config", "--services", "--quiet"],
        "neither --json nor the terminal touches a raw passthrough"
    );

    let cmd = DockerCmd::Run(RunArgs {
        args: vec!["restart".to_string(), "chap".to_string()],
    });
    assert_eq!(args_for(&cmd, TERM), vec!["restart", "chap"]);

    let cmd = DockerCmd::Run(RunArgs { args: vec![] });
    assert!(args_for(&cmd, TERM).is_empty());
}

#[test]
fn config_serialises_under_the_global_flag() {
    let cmd = DockerCmd::Config(ConfigArgs { extra: vec![] });
    assert_eq!(args_for(&cmd, TERM), vec!["config"]);
    assert_eq!(args_for(&cmd, JSON), vec!["config", "--format", "json"]);

    let cmd = DockerCmd::Config(ConfigArgs {
        extra: vec!["--services".to_string()],
    });
    assert_eq!(args_for(&cmd, TERM), vec!["config", "--services"]);
    assert_eq!(
        args_for(&cmd, JSON),
        vec!["config", "--format", "json", "--services"]
    );
}

#[test]
fn detect_keeps_the_json_flag_it_is_given() {
    assert!(Shell::detect(true).json);
    assert!(!Shell::detect(false).json);
}

/// One running container of `service`, created at `created`.
fn container(service: &str, id: &str, created: &str) -> docker::Container {
    docker::Container {
        service: service.to_string(),
        name: format!("chapx-{service}-1"),
        id: id.to_string(),
        created_at: created.to_string(),
        state: "running".to_string(),
        ..docker::Container::default()
    }
}

#[test]
fn up_summarises_what_it_started_and_what_it_left_alone() {
    let plain = Components::default();
    let summary = |before: &[docker::Container], after: &[docker::Container]| {
        up_summary(&Out::default(), before, after, &plain)
    };
    let before = vec![
        container("chap", "aaa", "t1"),
        container("postgres", "bbb", "t1"),
    ];
    let after = vec![
        container("chap", "ccc", "t2"),
        container("postgres", "bbb", "t1"),
        container("chapkit-ewars-model", "ddd", "t2"),
    ];
    assert_eq!(
        summary(&before, &after),
        "started/recreated: chap, chapkit-ewars-model; unchanged: postgres\n\
             run `chaps status` to check that everything answers"
    );

    // A first start has nothing to leave alone.
    assert!(
        summary(&[], &after).starts_with(
            "started/recreated: chap, postgres, chapkit-ewars-model\nrun `chaps status`"
        )
    );
    // A no-op `up` says so rather than printing an empty list.
    assert!(summary(&after, &after).starts_with("unchanged: chap, postgres, chapkit"));
    // And an `up` that left nothing running is a problem, not a summary.
    assert_eq!(
        summary(&[], &[]),
        "nothing is running after `up`; run `chaps logs` to see why"
    );
}

/// A deployment with a DHIS2 nothing has connected is told so by the one
/// command that has just spent minutes starting it - and stops being told
/// the moment a connect is recorded.
#[test]
fn up_names_the_connect_a_dhis2_deployment_still_needs() {
    let mut components = Components::default();
    components.set_enabled(crate::components::Component::Dhis2, true);
    let after = vec![
        container("chap", "aaa", "t1"),
        container("dhis2", "bbb", "t1"),
    ];

    let asked = up_summary(&Out::default(), &[], &after, &components);
    assert!(
        asked.ends_with(
            "\nchaps has not connected this DHIS2 to CHAP; \
                 run `chaps dhis2 connect` once DHIS2 answers"
        ),
        "{asked}"
    );
    // Under the line that is always there, not instead of it.
    assert!(
        asked.contains("run `chaps status` to check that everything answers"),
        "{asked}"
    );

    components.dhis2.connected_at = Some("2026-09-27T09:12:33Z".to_string());
    let recorded = up_summary(&Out::default(), &[], &after, &components);
    assert!(!recorded.contains("chaps dhis2 connect"), "{recorded}");

    // An `up` that started nothing has no deployment to connect, and the
    // line above already says to go and read the logs.
    components.dhis2.connected_at = None;
    assert_eq!(
        up_summary(&Out::default(), &[], &[], &components),
        "nothing is running after `up`; run `chaps logs` to see why"
    );

    // And a deployment without chap-core is never asked: the route would
    // point at a service that is not there.
    components.set_enabled(crate::components::Component::ChapCore, false);
    let standalone = up_summary(&Out::default(), &[], &after, &components);
    assert!(!standalone.contains("chaps dhis2 connect"), "{standalone}");
}

/// A mounted config written after its container was created is the edit
/// compose cannot see; one written before it, or a service not asked for,
/// is left to the plain run.
#[test]
fn a_config_edited_after_its_container_started_is_recreated() {
    let temp = tempfile::tempdir().unwrap();
    let project = Project {
        dir: temp.path().to_path_buf(),
        state: Default::default(),
    };
    std::fs::create_dir_all(temp.path().join("dhis2")).unwrap();
    std::fs::write(temp.path().join("dhis2/dhis.conf"), "x").unwrap();
    let container = |service: &str, created_at: &str| docker::Container {
        service: service.to_string(),
        created_at: created_at.to_string(),
        ..Default::default()
    };
    // Created long before the file was written: edited since.
    let old = [container("dhis2", "2020-01-01 00:00:00 +0000 UTC")];
    assert_eq!(edited_configs(&project, &old, &[]), vec!["dhis2"]);
    assert_eq!(
        edited_configs(&project, &old, &["dhis2".to_string()]),
        vec!["dhis2"]
    );
    assert!(edited_configs(&project, &old, &["ocs".to_string()]).is_empty());
    // Created after it: nothing to apply.
    let new = [container("dhis2", "2099-01-01 00:00:00 +0000 UTC")];
    assert!(edited_configs(&project, &new, &[]).is_empty());
    // Not running: nothing to recreate.
    assert!(edited_configs(&project, &[], &[]).is_empty());
}

/// `down --volumes` takes `dhis2_db` with it, and the next `chaps up`
/// restores the seed dump into a new one - which ships a `chap` route
/// pointing at somebody else's CHAP. So the record of a connect goes with
/// the volume it was true of, and the line says why.
#[test]
fn down_volumes_forgets_a_connect_recorded_for_the_database_it_removed() {
    let temp = tempfile::tempdir().unwrap();
    let mut project = Project {
        dir: temp.path().to_path_buf(),
        state: crate::project::ProjectState {
            compose_project: "demo-1ab2c3".to_string(),
            ..Default::default()
        },
    };
    project
        .state
        .components
        .set_enabled(crate::components::Component::Dhis2, true);
    project.state.components.dhis2.connected_at = Some("2026-09-27T09:12:33Z".to_string());
    project.save().unwrap();
    let db = project
        .prefixed_volume(crate::compose::render::DHIS2_DB_VOLUME)
        .unwrap();

    // A `down` that took other volumes and left the database leaves the
    // record where it was: the route it was true of is still there.
    assert_eq!(
        forget_dhis2_connect(&mut project, &["demo-1ab2c3_dhis2_home".to_string()]),
        None
    );
    assert!(project.state.components.dhis2.connected_at.is_some());

    let line = forget_dhis2_connect(&mut project, std::slice::from_ref(&db)).unwrap();
    assert!(line.contains("`dhis2_db`"), "{line}");
    assert!(line.contains("`chaps dhis2 connect`"), "{line}");
    assert_eq!(project.state.components.dhis2.connected_at, None);
    // And on disk, not only in memory: the next command reads the file.
    let reloaded = Project::load(temp.path()).unwrap();
    assert_eq!(reloaded.state.components.dhis2.connected_at, None);
    assert!(reloaded.state.components.dhis2_needs_connecting());

    // Nothing recorded, nothing to say - however many volumes went.
    assert_eq!(
        forget_dhis2_connect(&mut project, std::slice::from_ref(&db)),
        None
    );

    // An unseeded DHIS2 comes back empty, with no route to be wrong about.
    project.state.components.dhis2.seed = crate::components::Dhis2Seed::None;
    project.state.components.dhis2.connected_at = Some("2026-09-27T09:12:33Z".to_string());
    let line = forget_dhis2_connect(&mut project, std::slice::from_ref(&db)).unwrap();
    assert!(line.contains("an empty DHIS2"), "{line}");
    assert!(!line.contains("seed dump"), "{line}");
}

#[test]
fn down_reports_what_it_stopped_and_what_it_kept() {
    let stopped = vec!["chap".to_string(), "worker".to_string()];
    let name = Some("mychap-1ab2c3");
    let kept =
        |stopped: &[String], name| down_summary(&Out::default(), stopped, DownVolumes::Kept, name);
    // The volumes that stay behind are named after the compose project,
    // so the line says which prefix to look for them under - and the
    // command that would take them, spelled as it is typed.
    assert_eq!(
        kept(&stopped, name),
        "stopped: chap, worker (2 containers); volumes kept: mychap-1ab2c3_* \
             (`chaps down --volumes` removes them)"
    );
    assert!(kept(&stopped[..1], name).starts_with("stopped: chap (1 container);"));
    // A deployment whose name could not be worked out still gets the line.
    assert!(
        kept(&stopped, None).ends_with("volumes kept (`chaps down --volumes` removes them)"),
        "{}",
        kept(&stopped, None)
    );
    assert_eq!(kept(&[], name), "nothing was running");
}

#[test]
fn down_volumes_names_the_volumes_it_removed() {
    let stopped = vec!["chap".to_string(), "worker".to_string()];
    let name = Some("mychap-1ab2c3");
    let removed = |stopped: &[String], gone: &[String]| {
        down_summary(&Out::default(), stopped, DownVolumes::Removed(gone), name)
    };
    let gone = vec![
        "mychap-1ab2c3_chap-db".to_string(),
        "mychap-1ab2c3_chap-data".to_string(),
    ];
    assert_eq!(
        removed(&stopped, &gone),
        "stopped: chap, worker (2 containers); removed 2 volumes \
             (mychap-1ab2c3_chap-db, mychap-1ab2c3_chap-data)"
    );
    assert_eq!(
        removed(&stopped, &gone[..1]),
        "stopped: chap, worker (2 containers); removed 1 volume \
             (mychap-1ab2c3_chap-db)"
    );
    // A deployment that was not running still had volumes to remove, and
    // that is the half worth reporting.
    assert_eq!(
        removed(&[], &gone[..1]),
        "nothing was running; removed 1 volume (mychap-1ab2c3_chap-db)"
    );
    // Compose removes the volumes its files declare, so a run that
    // reached none of them says so rather than claiming a removal.
    assert_eq!(
        removed(&stopped, &[]),
        "stopped: chap, worker (2 containers); no volumes were removed"
    );
}

#[test]
fn down_names_what_brings_the_deployment_back() {
    assert!(down_next(false).contains("`chaps up`"));
    assert!(down_next(false).contains("with its data"));
    assert!(down_next(true).contains("`chaps up`"));
    assert!(down_next(true).contains("delete this folder"));
}

/// The command line as the process received it.
fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_volume_flag_after_down_is_refused_by_name() {
    // Clap resolves this one to --verbose, so only the raw arguments can
    // still say it was typed - and it has to be said, because compose
    // never saw it and the volumes are all still there.
    assert_eq!(
        misused_volumes_flag(&argv(&["chaps", "down", "-v"]), &[]),
        Some("-v")
    );
    assert_eq!(
        misused_volumes_flag(&argv(&["chaps", "-C", "/srv/chap", "down", "-v"]), &[]),
        Some("-v")
    );
    // Past a `--` it reaches EXTRA, where passing it through would
    // destroy the data of an operator who only asked for a louder `down`.
    for token in ["-v", "--volumes", "--volume"] {
        let extra = vec!["--timeout".to_string(), "5".to_string(), token.to_string()];
        let line = argv(&["chaps", "down", "--", "--timeout", "5", token]);
        assert_eq!(misused_volumes_flag(&line, &extra), Some(token), "{token}");
    }

    // `chaps -v down` asked for a verbose stop and gets one.
    assert_eq!(
        misused_volumes_flag(&argv(&["chaps", "-v", "down"]), &[]),
        None
    );
    // And the flag that does the job is not the mistake it replaces.
    assert_eq!(
        misused_volumes_flag(&argv(&["chaps", "down", "--volumes", "--yes"]), &[]),
        None
    );
    // Everything compose takes goes through untouched.
    assert_eq!(
        misused_volumes_flag(
            &argv(&["chaps", "down", "--", "--rmi", "local"]),
            &["--rmi".to_string(), "local".to_string()]
        ),
        None
    );
    assert_eq!(misused_volumes_flag(&[], &[]), None);
}

#[test]
fn the_refusal_names_both_flags_and_the_verbose_spelling() {
    for token in ["-v", "--volumes"] {
        let message = volumes_flag_message(token);
        assert!(
            message.starts_with(&format!("`{token}` after `down`")),
            "{message}"
        );
        assert!(message.contains("--verbose"), "{message}");
        assert!(message.contains("`chaps down --volumes`"), "{message}");
        assert!(message.contains("`chaps -v down`"), "{message}");
    }
}

#[test]
fn down_volumes_asks_compose_for_the_volumes_and_the_orphans() {
    assert_eq!(
        args_for(&down(true, &[]), TERM),
        vec!["down", "-v", "--remove-orphans"]
    );
    // Once, even when the caller asked for the orphans as well.
    assert_eq!(
        args_for(&down(true, &["--remove-orphans"]), TERM),
        vec!["down", "-v", "--remove-orphans"]
    );
}

#[test]
fn the_volumes_at_stake_are_named_before_they_are_removed() {
    let volumes = vec![
        "mychap-1ab2c3_chap-db".to_string(),
        "mychap-1ab2c3_ocs_data".to_string(),
    ];
    let prefix = Some("mychap-1ab2c3_");
    assert_eq!(
        volumes_at_stake(&Out::default(), &volumes, prefix, None),
        "docker holds 2 volumes under mychap-1ab2c3_*, with their data: \
             mychap-1ab2c3_chap-db, mychap-1ab2c3_ocs_data"
    );
    assert_eq!(
        volumes_at_stake(&Out::default(), &volumes[..1], prefix, None),
        "docker holds 1 volume under mychap-1ab2c3_*, with their data: mychap-1ab2c3_chap-db"
    );
    // A deployment that was never started, or a docker that could not be
    // asked: either way the prompt says what is known before it asks.
    assert_eq!(
        volumes_at_stake(&Out::default(), &[], prefix, None),
        "docker holds no volumes under mychap-1ab2c3_*"
    );
    assert_eq!(
        volumes_at_stake(&Out::default(), &[], None, None),
        "docker holds no volumes under this deployment's name"
    );

    // What the OCS volume holds, when the `du` could be had: the number
    // that decides whether this prompt gets a yes.
    assert_eq!(
        volumes_at_stake(
            &Out::default(),
            &volumes,
            prefix,
            Some(("mychap-1ab2c3_ocs_data", 3 * 1024 * 1024 * 1024))
        ),
        "docker holds 2 volumes under mychap-1ab2c3_*, with their data: \
             mychap-1ab2c3_chap-db, mychap-1ab2c3_ocs_data \
             (mychap-1ab2c3_ocs_data holds 3.0 GB)"
    );
    // A volume docker does not hold is nothing to size up.
    assert_eq!(
        ocs_data_at_stake("mychap-1ab2c3_", &volumes[..1]),
        None,
        "no ocs_data volume, no du"
    );
}

#[test]
fn only_the_wrappers_that_can_hit_a_failed_dependency_read_stderr() {
    // A detached `up` and `restart` can end on "dependency failed to
    // start", and compose says which container that was on stderr.
    assert!(reads_stderr(&up(false, &[])));
    assert!(reads_stderr(&restart(false, &[])));
    // An attached `up` is streaming logs; its streams stay docker's own.
    assert!(!reads_stderr(&up(true, &[])));
    assert!(!reads_stderr(&down(false, &[])));
    assert!(!reads_stderr(&exec("chap", &[])));
}

#[test]
fn pull_counts_the_images_it_fetched() {
    assert_eq!(pull_summary(Some(4)), "pulled 4 images");
    assert_eq!(pull_summary(Some(1)), "pulled 1 image");
    assert_eq!(pull_summary(Some(0)), "pulled 0 images");
    assert_eq!(
        pull_summary(None),
        "pulled the images this project pins",
        "the pull worked even when the count could not be read"
    );
}

#[test]
fn logs_names_the_services_there_are() {
    let services = vec![
        "chap".to_string(),
        "chapkit-ewars-model".to_string(),
        "postgres".to_string(),
    ];
    assert_eq!(
        unknown_service_message(&["chap-worker".to_string()], &services),
        "no service `chap-worker` in this project; the services are \
             chap, chapkit-ewars-model, postgres"
    );
    assert!(
        unknown_service_message(&["a".to_string(), "b".to_string()], &services)
            .starts_with("no service `a`, `b` in this project;")
    );
}

#[test]
fn the_empty_state_line_says_what_to_do_about_it() {
    assert_eq!(
        NOTHING_RUNNING,
        "nothing is running for this project; start CHAP with `chaps up`"
    );
}
