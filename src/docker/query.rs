//! What compose says about a deployment's files: its services, images,
//! configuration hashes and logs, and the other compose projects on this
//! machine.

use super::{compose_capture, docker_capture, ps_entries};
use crate::project::Project;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The services this project's compose files define
/// (`docker compose config --services`), or `None` when docker could not be
/// asked.
pub fn config_services(project: &Project) -> Option<Vec<String>> {
    let text = compose_capture(project, &["config", "--services"])?;
    let mut names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    Some(names)
}

/// How many distinct images this project's stack pins
/// (`docker compose config --images`), or `None` when docker could not be
/// asked. chap and its worker share one image, so the list is deduplicated.
pub fn image_count(project: &Project) -> Option<usize> {
    let text = compose_capture(project, &["config", "--images"])?;
    let images: BTreeSet<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    Some(images.len())
}

/// The image reference each service of this project pins, from
/// `docker compose config --format json`.
///
/// This is the stack as the files describe it now, which is the half
/// `docker compose ps` cannot answer: `ps` reports the reference a container
/// was created from, and the point of asking both is to find where they have
/// come apart. Best-effort: an empty map when docker could not be asked.
pub fn service_images(project: &Project) -> BTreeMap<String, String> {
    match compose_capture(project, &["config", "--format", "json"]) {
        Some(text) => parse_service_images(&text),
        None => BTreeMap::new(),
    }
}

/// The `services.<name>.image` of a `docker compose config --format json`
/// document. A service built from a Dockerfile has no image and is left out.
pub fn parse_service_images(text: &str) -> BTreeMap<String, String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return BTreeMap::new();
    };
    let Some(services) = value.get("services").and_then(|s| s.as_object()) else {
        return BTreeMap::new();
    };
    services
        .iter()
        .filter_map(|(name, service)| {
            let image = service.get("image")?.as_str()?;
            (!image.is_empty()).then(|| (name.clone(), image.to_string()))
        })
        .collect()
}

/// The configuration hash compose computes for each service of this project
/// right now (`docker compose config --hash='*'`).
///
/// It is the same value compose stamps on a container as
/// `com.docker.compose.config-hash` when it creates it, so the two together
/// say whether a running container was made from the files as they are today.
/// Best-effort: an empty map when docker could not be asked.
pub fn config_hashes(project: &Project) -> BTreeMap<String, String> {
    match compose_capture(project, &["config", "--hash=*"]) {
        Some(text) => parse_config_hashes(&text),
        None => BTreeMap::new(),
    }
}

/// Parse the `<service> <hash>` lines of `docker compose config --hash='*'`.
pub fn parse_config_hashes(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (service, hash) = line.trim().split_once(char::is_whitespace)?;
            let hash = hash.trim();
            (!service.is_empty() && !hash.is_empty())
                .then(|| (service.to_string(), hash.to_string()))
        })
        .collect()
}

/// The last `tail` lines one service has logged
/// (`docker compose logs --tail N --no-color SERVICE`).
///
/// `--no-color` because this is read rather than shown: the escape codes
/// compose adds to colour the service column would end up inside the lines
/// that are quoted back. Best-effort: `None` when docker could not be asked.
pub fn service_logs(project: &Project, service: &str, tail: usize) -> Option<String> {
    let tail = tail.to_string();
    compose_capture(
        project,
        &[
            "logs",
            "--tail",
            &tail,
            "--no-color",
            "--no-log-prefix",
            service,
        ],
    )
    // `--no-log-prefix` arrived in compose 2.x but not in every 2.x; a
    // compose that rejects it still answers the same question without it.
    .or_else(|| compose_capture(project, &["logs", "--tail", &tail, "--no-color", service]))
}

/// The comment on the database of the DHIS2 component, which is the mark a
/// complete seed restore leaves. `None` when docker or psql could not answer;
/// an empty string when the database has no comment.
pub fn dhis2_seed_mark(project: &Project) -> Option<String> {
    compose_capture(
        project,
        &[
            "exec",
            "-T",
            "dhis2-db",
            "sh",
            "-c",
            "psql -U \"$POSTGRES_USER\" -d \"$POSTGRES_DB\" -tAc \
             \"SELECT shobj_description(oid, 'pg_database') FROM pg_database \
             WHERE datname = current_database()\"",
        ],
    )
    .map(|answer| answer.trim().to_string())
}

/// Every compose project this docker has seen, running or not, as
/// `docker compose ls -a --format json` prints it.
///
/// Best-effort like every other query here, and quiet about it: `None` when
/// docker is not installed, the daemon is not up or it answered non-zero. The
/// one caller only wants a hint, and `varde init` has to work on a machine
/// that has never run docker at all.
pub fn compose_ls_json() -> Option<String> {
    docker_capture(&[
        "compose".to_string(),
        "ls".to_string(),
        "-a".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ])
}

/// The name of a running container that publishes host `port`, if any.
///
/// Best-effort like [`compose_ls_json`]: `None` when docker cannot be asked.
pub fn container_publishing(port: u16) -> Option<String> {
    let text = docker_capture(&[
        "ps".to_string(),
        "--filter".to_string(),
        format!("publish={port}"),
        "--format".to_string(),
        "{{.Names}}".to_string(),
    ])?;
    text.lines()
        .map(str::trim)
        .find(|name| !name.is_empty())
        .map(str::to_string)
}

/// The varde deployment directories named by [`compose_ls_json`].
///
/// A compose project is one of ours when its `ConfigFiles` - a comma-separated
/// list of absolute paths - names a [`crate::project::VARDE_COMPOSE`]; that
/// file's directory is the deployment. A project whose directory has since
/// been deleted is left out, since there is nothing left to warn about, and so
/// is anything that is not JSON at all.
pub fn compose_ls_dirs(text: &str) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for value in ps_entries(text) {
        let Some(files) = value.get("ConfigFiles").and_then(|f| f.as_str()) else {
            continue;
        };
        for file in files.split(',') {
            let path = Path::new(file.trim());
            if path.file_name() != Some(OsStr::new(crate::project::VARDE_COMPOSE)) {
                continue;
            }
            let Some(dir) = path.parent().filter(|dir| dir.is_dir()) else {
                continue;
            };
            if !dirs.iter().any(|seen| seen == dir) {
                dirs.push(dir.to_path_buf());
            }
        }
    }
    dirs
}
