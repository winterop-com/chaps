//! The parts of a backup that come out of Docker: the database dump and one
//! tar per model and component data volume.

use super::quiesce::{Quiesce, Running};
use crate::backup::{self, DB_MEMBER, ManifestComponent, ManifestDatabase, ManifestModel, Stage};
use crate::compose::volume_name;
use crate::docker;
use crate::error::Result;
use crate::project::{ENV_FILE, Project};

/// `pg_dump -Fc` the chap-core database into the stage.
///
/// postgres has to be up: the dump goes through `docker compose exec`, which
/// needs a running container. `--no-db` is the way out when it is not.
///
/// Nothing is paused for this one: `pg_dump` reads a single transactional
/// snapshot, so the dump is consistent however busy chap-core is while it
/// runs. That is also why the database is dumped rather than its volume
/// tarred.
pub(super) fn dump_database(
    project: &Project,
    stage: &Stage,
    running: &mut Running,
) -> Result<ManifestDatabase> {
    if !running.has("postgres") {
        return Err(anyhow::anyhow!(
            "the postgres container is not running, so the database cannot be dumped; \
             start it with `chaps up`, or pass --no-db"
        ));
    }
    let env_body = std::fs::read_to_string(project.dir.join(ENV_FILE)).unwrap_or_default();
    let (user, name) = backup::postgres_credentials(&env_body);

    let dest = stage.path(DB_MEMBER)?;
    let args = compose_exec("postgres", &["pg_dump", "-U", &user, "-Fc", &name]);
    let piped = docker::run_compose_piped(project, &args, None, Some(&dest))?;
    if piped.code != 0 {
        return Err(anyhow::anyhow!(
            "pg_dump failed (exit {}): {}",
            piped.code,
            backup::first_line(&piped.stderr)
        ));
    }
    Ok(ManifestDatabase {
        path: DB_MEMBER.to_string(),
        server_version: server_version(project, &user, &name),
        user,
        name,
        size_bytes: backup::file_size(&dest),
    })
}

/// What the server calls itself, for the manifest. Best effort: a backup is
/// not worth failing over a label.
///
/// The database has to be named: `psql -U chap` alone would connect to a
/// database called `chap`, which chap-core does not have.
fn server_version(project: &Project, user: &str, name: &str) -> Option<String> {
    let args = compose_exec(
        "postgres",
        &[
            "psql",
            "-U",
            user,
            "-d",
            name,
            "-tAc",
            "show server_version",
        ],
    );
    let (code, stdout, _) = docker::compose_output(project, &args).ok()?;
    if code != 0 {
        return None;
    }
    let version = stdout.trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// One tar per enabled model, read out of its data volume while the service
/// is held still.
pub(super) fn capture_models(
    project: &Project,
    stage: &Stage,
    running: &mut Running,
    skip: bool,
) -> Result<Vec<ManifestModel>> {
    let mut out = Vec::new();
    if project.state.models.is_empty() {
        return Ok(out);
    }
    // The volume names carry the compose project name; without it there is no
    // way to tell "never started" from "empty", so every model is attempted.
    let prefix = if skip {
        None
    } else {
        project.compose_project_name()
    };

    for (id, model) in &project.state.models {
        let volume = volume_name(id);
        let mut entry = ManifestModel {
            id: id.clone(),
            service_id: model.service_id.clone(),
            version: model.version.clone(),
            image_tag: model.image_tag.clone(),
            host_port: model.host_port,
            data_dir: model.data_dir.clone(),
            user: model.user.clone(),
            volume: volume.clone(),
            path: None,
            size_bytes: 0,
            skipped: None,
            failed: false,
            quiesce: None,
        };
        if skip {
            entry.skipped = Some("--no-models".to_string());
            out.push(entry);
            continue;
        }
        if let Some(prefix) = &prefix
            && !docker::volume_exists(&format!("{prefix}_{volume}"))
        {
            entry.skipped = Some(format!(
                "no {volume} volume yet, so there is no data to back up; \
                 the service has never started"
            ));
            out.push(entry);
            continue;
        }

        let member = backup::model_member(&model.service_id);
        let dest = stage.path(&member)?;
        // The init service mounts the same volume at the same path as the model
        // and does nothing else, so this reads the data whether the model is
        // running, stopped or has never been started at all.
        let args = compose_run(
            &format!("{}-init", model.service_id),
            &["tar", "cf", "-", "-C", &model.data_dir, "."],
        );
        // The model's own service is frozen for the read: a chapkit service
        // keeps a live SQLite database in its data directory, and a tar taken
        // while something writes to one is a tar of a torn database.
        let mut quiesce = Quiesce::hold(project, &model.service_id, running.has(&model.service_id));
        let piped = docker::run_compose_piped(project, &args, None, Some(&dest));
        entry.quiesce = quiesce.release();
        let piped = piped?;
        if piped.code != 0 {
            let _ = std::fs::remove_file(&dest);
            entry.failed = true;
            entry.skipped = Some(format!(
                "reading {} failed (exit {}): {}",
                model.data_dir,
                piped.code,
                backup::first_line(&piped.stderr)
            ));
            out.push(entry);
            continue;
        }
        entry.size_bytes = backup::file_size(&dest);
        entry.path = Some(member);
        out.push(entry);
    }
    Ok(out)
}

/// One tar per enabled component with state of its own, read out of its named
/// volume while the service is held still.
///
/// No component one-shot mounts a volume the archive holds, so each one is
/// mounted into a throwaway busybox container instead - the one command in
/// a backup that is a plain `docker run` rather than a compose one, because
/// there is no compose service that would do it.
pub(super) fn capture_components(
    project: &Project,
    stage: &Stage,
    running: &mut Running,
    skip: bool,
) -> Result<Vec<ManifestComponent>> {
    let mut out = Vec::new();
    let parts = backup::component_volumes(&project.state.components);
    if parts.is_empty() {
        return Ok(out);
    }
    // The volume names carry the compose project name, exactly as for a model.
    let prefix = if skip {
        None
    } else {
        project.compose_project_name()
    };

    for part in parts {
        let mut entry = ManifestComponent {
            name: part.name.to_string(),
            service: part.service.to_string(),
            volume: part.volume.to_string(),
            data_dir: part.data_dir.to_string(),
            path: None,
            size_bytes: 0,
            skipped: None,
            failed: false,
            quiesce: None,
        };
        if skip {
            entry.skipped = Some("--no-components".to_string());
            out.push(entry);
            continue;
        }
        let Some(prefix) = &prefix else {
            entry.skipped = Some(
                "the compose project name could not be read, so the volume cannot be \
                 named; is Docker running?"
                    .to_string(),
            );
            out.push(entry);
            continue;
        };
        let volume = format!("{prefix}_{}", part.volume);
        if !docker::volume_exists(&volume) {
            entry.skipped = Some(format!(
                "no {} volume yet, so there is no data to back up; \
                 the service has never started",
                part.volume
            ));
            out.push(entry);
            continue;
        }

        let member = backup::component_member(part.member);
        let dest = stage.path(&member)?;
        let mut quiesce = Quiesce::hold(project, part.service, running.has(part.service));
        let read = backup::read_volume(&volume, &dest);
        entry.quiesce = quiesce.release();
        let piped = read?;
        if piped.code != 0 {
            let _ = std::fs::remove_file(&dest);
            entry.failed = true;
            entry.skipped = Some(format!(
                "reading {volume} failed (exit {}): {}",
                piped.code,
                backup::first_line(&piped.stderr)
            ));
            out.push(entry);
            continue;
        }
        entry.size_bytes = backup::file_size(&dest);
        entry.path = Some(member);
        out.push(entry);
    }
    Ok(out)
}

/// `exec -T <service> <cmd..>`, the form that needs no terminal.
pub(super) fn compose_exec(service: &str, cmd: &[&str]) -> Vec<String> {
    let mut args = vec!["exec".to_string(), "-T".to_string(), service.to_string()];
    args.extend(cmd.iter().map(|s| s.to_string()));
    args
}

/// `run --rm --no-deps -T <service> <cmd..>`: a throwaway container that
/// starts nothing else, so a stopped stack stays stopped.
pub(super) fn compose_run(service: &str, cmd: &[&str]) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--no-deps".to_string(),
        "-T".to_string(),
        service.to_string(),
    ];
    args.extend(cmd.iter().map(|s| s.to_string()));
    args
}
