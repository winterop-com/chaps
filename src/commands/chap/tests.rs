use super::files::{Snapshot, describe};
use super::plan::*;
use crate::cli::ChapImage;
use std::path::{Path, PathBuf};

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}

#[test]
fn the_model_name_is_read_in_both_spellings() {
    let args = strings(&["eval", "--model-name", "http://m:8000", "--x", "1"]);
    assert_eq!(model_name(&args), Some("http://m:8000"));
    let args = strings(&["eval", "--model-name=./my_model"]);
    assert_eq!(model_name(&args), Some("./my_model"));
    assert_eq!(model_name(&strings(&["plot-backtest", "x.nc"])), None);
}

#[test]
fn a_model_name_is_a_chapkit_service_a_repository_or_a_directory() {
    assert_eq!(model_kind(None), ModelKind::None);
    assert_eq!(
        model_kind(Some("http://auto-arima:8000")),
        ModelKind::Chapkit("http://auto-arima:8000".to_string())
    );
    assert_eq!(
        model_kind(Some("https://github.com/dhis2-chap/Minimalist_R@a1b2c3")),
        ModelKind::GitHub {
            owner: "dhis2-chap".to_string(),
            repo: "Minimalist_R".to_string(),
            git_ref: Some("a1b2c3".to_string()),
        }
    );
    assert_eq!(
        model_kind(Some("https://www.github.com/o/r.git/")),
        ModelKind::GitHub {
            owner: "o".to_string(),
            repo: "r".to_string(),
            git_ref: None,
        }
    );
    assert_eq!(
        model_kind(Some("models/ewars")),
        ModelKind::Local(PathBuf::from("models/ewars"))
    );
}

#[test]
fn only_the_models_that_are_not_chapkit_services_need_the_worker_image() {
    let repo = model_kind(Some("https://github.com/o/r"));
    let chapkit = model_kind(Some("http://m:8000"));
    assert_eq!(image_for(&ModelKind::None, false), ChapImage::Core);
    assert_eq!(image_for(&chapkit, false), ChapImage::Core);
    assert_eq!(image_for(&repo, false), ChapImage::Worker);
    assert_eq!(
        image_for(&ModelKind::Local("m".into()), false),
        ChapImage::Worker
    );
    // A deployment has the worker image already, so it runs everything.
    assert_eq!(image_for(&chapkit, true), ChapImage::Worker);
}

#[test]
fn docker_env_is_found_in_an_mlproject_and_only_there() {
    let docker = "name: ewars\n\ndocker_env:\n  image: ivargr/r_inla:latest\n";
    assert!(uses_docker_env(docker));
    assert!(!uses_docker_env("name: x\nrenv_env: renv.lock\n"));
    assert!(!uses_docker_env(
        "name: x\n# docker_env: no longer\nuv_env: pyproject.toml\n"
    ));
}

#[test]
fn the_newest_tag_is_the_highest_release() {
    let tags = strings(&["master", "v2.3.1", "v2.10.0", "latest", "v2.9.9"]);
    assert_eq!(newest_tag(&tags).as_deref(), Some("v2.10.0"));
    assert_eq!(
        newest_tag(&strings(&["master", "latest"])).as_deref(),
        Some("master")
    );
    assert_eq!(newest_tag(&[]), None);
}

// Unix paths: the Windows builds ship untested.
#[cfg(unix)]
#[test]
fn only_paths_outside_the_current_directory_need_a_mount() {
    let cwd = Path::new("/home/u/work");
    let args = strings(&[
        "eval",
        "--dataset-csv",
        "data.csv",
        "--output-file",
        "/data/out/x.nc",
        "--plot-file=../reports/x.html",
        "--model-name",
        "https://github.com/o/r",
        "./inside/a.nc",
        "/home/u/work/also-inside.nc",
    ]);
    assert_eq!(
        outside_paths(&args, cwd),
        vec![
            PathBuf::from("/data/out/x.nc"),
            PathBuf::from("/home/u/reports/x.html"),
        ]
    );
}

// Unix paths: the Windows builds ship untested.
#[cfg(unix)]
#[test]
fn the_nearest_existing_directory_is_mounted_once() {
    let exists = |path: &Path| {
        [
            "/data",
            "/data/out",
            "/home",
            "/home/u",
            "/home/u/work",
            "/srv",
        ]
        .iter()
        .any(|dir| path == Path::new(dir))
    };
    let paths = vec![
        PathBuf::from("/data/out/x.nc"),
        PathBuf::from("/data/out/new/y.nc"),
        PathBuf::from("/data/z.csv"),
        PathBuf::from("/home/u/work/sub/a.nc"),
        PathBuf::from("/srv/missing/deep/b.nc"),
    ];
    let covered = vec![PathBuf::from("/home/u/work")];
    assert_eq!(
        mount_dirs(&paths, &covered, exists).unwrap(),
        vec![PathBuf::from("/data"), PathBuf::from("/srv")]
    );
}

// Unix paths: the Windows builds ship untested.
#[cfg(unix)]
#[test]
fn the_root_directory_is_never_mounted() {
    let none_exist = |path: &Path| path == Path::new("/");
    let err = mount_dirs(&[PathBuf::from("/nowhere/x.nc")], &[], none_exist).unwrap_err();
    assert_eq!(err, PathBuf::from("/nowhere/x.nc"));
}

fn spec() -> RunSpec {
    RunSpec {
        image: "ghcr.io/dhis2-chap/chap-core:v2.3.1".to_string(),
        cwd: PathBuf::from("/home/u/work"),
        mounts: vec![PathBuf::from("/data")],
        data: PathBuf::from("/home/u/.local/share/chaps/chap"),
        user: Some((1000, 1000)),
        network: None,
        docker: None,
        tty: false,
        args: strings(&["eval", "--model-name", "http://m:8000"]),
    }
}

/// The value after `flag`, every time it occurs.
fn values<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
    args.windows(2)
        .filter(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
        .collect()
}

// Unix paths: the Windows builds ship untested.
#[cfg(unix)]
#[test]
fn a_run_mounts_every_directory_at_the_same_path_and_runs_as_the_user() {
    let args = docker_args(&spec());
    assert_eq!(args[..3], strings(&["run", "--rm", "-i"])[..]);
    assert!(!args.contains(&"-t".to_string()));
    assert_eq!(values(&args, "--user"), ["1000:1000"]);
    assert_eq!(values(&args, "--platform"), ["linux/amd64"]);
    assert_eq!(
        values(&args, "-v"),
        [
            "/home/u/work:/home/u/work",
            "/data:/data",
            "/home/u/.local/share/chaps/chap:/home/u/.local/share/chaps/chap",
        ]
    );
    assert_eq!(values(&args, "-w"), ["/home/u/work"]);
    let env = values(&args, "-e");
    assert!(env.contains(&"CHAP_RUNS_DIR=/home/u/.local/share/chaps/chap/runs"));
    assert!(env.contains(&"RENV_CONFIG_SANDBOX_ENABLED=FALSE"));
    assert!(values(&args, "--network").is_empty());
    assert_eq!(
        args[args.len() - 5..],
        strings(&[
            "ghcr.io/dhis2-chap/chap-core:v2.3.1",
            "chap",
            "eval",
            "--model-name",
            "http://m:8000",
        ])[..]
    );
}

// Unix paths: the Windows builds ship untested.
#[cfg(unix)]
#[test]
fn a_terminal_a_network_and_the_socket_are_added_when_asked_for() {
    let mut spec = spec();
    spec.tty = true;
    spec.network = Some("demo-ab12cd_default".to_string());
    spec.docker = Some(Some(0));
    spec.user = None;
    let args = docker_args(&spec);
    assert!(args.contains(&"-t".to_string()));
    assert_eq!(values(&args, "--network"), ["demo-ab12cd_default"]);
    assert!(values(&args, "-v").contains(&"/var/run/docker.sock:/var/run/docker.sock"));
    assert_eq!(values(&args, "--group-add"), ["0"]);
    assert!(values(&args, "--user").is_empty());

    // A socket whose group could not be read is still mounted.
    spec.docker = Some(None);
    assert!(values(&docker_args(&spec), "--group-add").is_empty());
}

#[test]
fn no_arguments_ask_chap_for_its_help() {
    let mut spec = spec();
    spec.args = Vec::new();
    let args = docker_args(&spec);
    assert_eq!(args[args.len() - 2..], strings(&["chap", "--help"])[..]);
}

#[test]
fn the_written_files_are_the_new_and_the_changed_ones() {
    let before = Snapshot::from_entries(&[("/w/data.csv", 10), ("/w/old.nc", 10)]);
    let after =
        Snapshot::from_entries(&[("/w/data.csv", 10), ("/w/old.nc", 20), ("/w/new.html", 20)]);
    assert_eq!(
        before.written(&after),
        vec![PathBuf::from("/w/new.html"), PathBuf::from("/w/old.nc")]
    );
    assert!(after.written(&after).is_empty());
}

// Unix paths: the Windows builds ship untested.
#[cfg(unix)]
#[test]
fn the_written_files_are_named_relative_to_the_current_directory() {
    let written = vec![
        PathBuf::from("/w/a.nc"),
        PathBuf::from("/w/out/b.html"),
        PathBuf::from("/data/c.csv"),
    ];
    assert_eq!(
        describe(&written, Path::new("/w"), 8),
        "a.nc, out/b.html, /data/c.csv"
    );
    assert_eq!(describe(&written, Path::new("/w"), 1), "a.nc, and 2 more");
}
