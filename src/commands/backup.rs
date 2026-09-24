//! `chaps backup create` — write a deployment into one `tar.gz`.
//!
//! Four parts, each skippable: the project files (a plain copy), the chap-core
//! database (`pg_dump -Fc` through the running postgres container), one tar
//! per model data volume (read by the overlay's one-shot init container, which
//! mounts the same volume the model does) and one tar per component data
//! volume (read through a busybox container, since `ocs` and `s3` have no init
//! container of their own). Everything is staged under `.chaps/tmp/`, packed
//! in one `tar -czf` into a temporary sibling of the destination and renamed
//! into place, so a failure halfway leaves no half-written archive and any
//! archive already at that path exactly as it was.
//!
//! A service that is running is paused for the seconds its volume takes to
//! read: a model keeps a live SQLite database in there, and tar reading a file
//! that is being written to produces a tar of a torn database. `pg_dump` needs
//! none of that - it reads one transactional snapshot - and a service that is
//! not running cannot write, so neither is disturbed.

use crate::backup::{
    self, COMPONENTS_MEMBER, DB_MEMBER, FILES_MEMBER, MANIFEST_MEMBER, MODELS_MEMBER, Manifest,
    ManifestComponent, ManifestDatabase, ManifestModel, Stage,
};
use crate::cli::BackupCreateArgs;
use crate::commands::Ctx;
use crate::compose::volume_name;
use crate::docker;
use crate::error::Result;
use crate::output::{self, Out};
use crate::project::{ENV_FILE, Project};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The shape of `chaps backup create --json`.
#[derive(Debug, Serialize)]
pub struct BackupReport {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub manifest: Manifest,
}

/// Stage, capture and pack.
pub fn run(ctx: &Ctx, args: &BackupCreateArgs) -> Result<()> {
    let project = ctx.project()?;
    let out = destination(&project, args.out.as_deref())?;

    let stage = Stage::new(&project.chaps_dir(), "backup")?;
    let mut members = vec![MANIFEST_MEMBER.to_string()];

    // Files. Always: without them the archive says nothing about what it is a
    // backup of, and they are the cheapest part by orders of magnitude.
    let files = backup::project_files(&project.dir);
    for rel in &files {
        let dst = stage.path(&format!("{FILES_MEMBER}/{rel}"))?;
        std::fs::copy(backup::join_relative(&project.dir, rel), &dst)
            .map_err(|e| anyhow::anyhow!("copying {rel}: {e}"))?;
    }
    if !files.is_empty() {
        members.push(FILES_MEMBER.to_string());
    }

    let mut running = Running::new(&project);
    let database = if args.no_db {
        None
    } else {
        let dumped = dump_database(&project, &stage, &mut running)?;
        // The dump is `db/chap_core.dump`; the directory is what tar is given.
        members.push("db".to_string());
        Some(dumped)
    };

    let models = capture_models(&project, &stage, &mut running, args.no_models)?;
    if models.iter().any(|m| m.path.is_some()) {
        members.push(MODELS_MEMBER.to_string());
    }

    let components = capture_components(&project, &stage, &mut running, args.no_components)?;
    if components.iter().any(|c| c.path.is_some()) {
        members.push(COMPONENTS_MEMBER.to_string());
    }

    let manifest = Manifest {
        schema_version: backup::SCHEMA_VERSION,
        created_by: format!("chaps-cli {}", ctx.cli_version),
        created_at: backup::timestamp(backup::now()),
        project: project_name(&project.dir),
        chap_image_tag: project.state.chap_image_tag.clone(),
        files: files.clone(),
        database,
        models,
        components,
    };
    std::fs::write(
        stage.dir.join(MANIFEST_MEMBER),
        backup::render_manifest(&manifest)?,
    )
    .map_err(|e| anyhow::anyhow!("writing the manifest: {e}"))?;

    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", parent.display()))?;
    }
    write_archive(&out, &stage.dir, &members)?;

    for model in manifest.models.iter().filter(|m| m.skipped.is_some()) {
        output::warn(&format!(
            "{}: {}",
            model.service_id,
            model.skipped.as_deref().unwrap_or("skipped")
        ));
    }
    for part in manifest.components.iter().filter(|c| c.skipped.is_some()) {
        output::warn(&format!(
            "{}: {}",
            part.name,
            part.skipped.as_deref().unwrap_or("skipped")
        ));
    }

    let report = BackupReport {
        size_bytes: backup::file_size(&out),
        path: out,
        manifest,
    };
    ctx.out.emit(&report, || human(&report, &ctx.out))
}

/// Pack the stage into a temporary sibling of `out` and rename it into place.
///
/// tar writes straight into the file it is given, so packing into `out` would
/// truncate whatever is there before it knows whether it can finish: a backup
/// that fails halfway would take last night's with it. The temporary file is
/// removed on failure, and `out` is then exactly as it was found - missing, or
/// the older archive, untouched.
fn write_archive(out: &Path, stage: &Path, members: &[String]) -> Result<()> {
    let tmp = backup::temp_archive_path(out);
    let packed = backup::tar_create(&tmp, stage, members).and_then(|()| {
        std::fs::rename(&tmp, out)
            .map_err(|e| anyhow::anyhow!("renaming {} to {}: {e}", tmp.display(), out.display()))
    });
    if packed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    packed
}

/// `docker compose ps`, asked at most once per run and only when something
/// needs to know.
///
/// Three parts of a backup do: the database dump refuses without a postgres,
/// and both volume captures pause whatever is running while they read. A run
/// that captures none of them asks Docker nothing at all, which is what makes
/// `--no-db --no-models --no-components` work on a machine with no Docker.
struct Running<'a> {
    project: &'a Project,
    asked: Option<BTreeSet<String>>,
}

impl<'a> Running<'a> {
    fn new(project: &'a Project) -> Running<'a> {
        Running {
            project,
            asked: None,
        }
    }

    fn services(&mut self) -> &BTreeSet<String> {
        let project = self.project;
        self.asked
            .get_or_insert_with(|| docker::running_services(project))
    }

    fn has(&mut self, service: &str) -> bool {
        self.services().contains(service)
    }
}

/// How a service was held still while its data volume was read.
#[derive(Debug, Clone, Copy)]
enum Held {
    /// `docker compose pause`: the processes are frozen, the container stays.
    Paused,
    /// `docker compose stop`, for a Docker with no pause.
    Stopped,
}

impl Held {
    fn verb(self) -> &'static str {
        match self {
            Held::Paused => "paused",
            Held::Stopped => "stopped",
        }
    }

    /// The compose command that lets the service go again.
    fn release(self) -> &'static str {
        match self {
            Held::Paused => "unpause",
            Held::Stopped => "start",
        }
    }
}

/// A running service held still for as long as its volume takes to read.
///
/// `docker compose pause` sends the container's processes `SIGSTOP`, so
/// nothing in it can write while tar reads - which is the whole point, because
/// a model keeps a live SQLite database in its data directory and tar has no
/// idea it is being written to. The service is always let go again, including
/// when the read fails: [`Drop`] releases whatever [`Quiesce::release`] has
/// not.
struct Quiesce<'a> {
    project: &'a Project,
    service: String,
    held: Option<Held>,
    since: Instant,
}

impl<'a> Quiesce<'a> {
    /// Hold `service` still, when it is running.
    ///
    /// A service that is not running cannot write, so nothing is done and
    /// nothing is reported. Where `pause` is unsupported - Windows containers,
    /// some rootless setups - stopping the service is slower but just as
    /// still; where neither works the read goes ahead with a warning, because
    /// a backup of a possibly-torn volume beats no backup at all.
    fn hold(project: &'a Project, service: &str, running: bool) -> Quiesce<'a> {
        let mut held = None;
        if running {
            held = match compose_step(project, &["pause", service]) {
                Ok(()) => Some(Held::Paused),
                Err(pause) => match compose_step(project, &["stop", service]) {
                    Ok(()) => Some(Held::Stopped),
                    Err(stop) => {
                        output::warn(&format!(
                            "{service} could not be held still, so its data is read while the \
                             service may be writing to it: {pause}; {stop}"
                        ));
                        None
                    }
                },
            };
        }
        Quiesce {
            project,
            service: service.to_string(),
            held,
            since: Instant::now(),
        }
    }

    /// Let the service go, and say how long it was held: `paused for 1.4 s`.
    fn release(&mut self) -> Option<String> {
        let held = self.held.take()?;
        let note = quiesce_note(held.verb(), self.since.elapsed());
        if let Err(why) = compose_step(self.project, &[held.release(), &self.service]) {
            output::warn(&format!(
                "{} was {} for the backup and could not be started again: {why}; \
                 run `chaps docker run -- {} {}`",
                self.service,
                held.verb(),
                held.release(),
                self.service
            ));
        }
        Some(note)
    }
}

impl Drop for Quiesce<'_> {
    fn drop(&mut self) {
        // Nothing to do when release() already ran; everything to do when the
        // read failed and the `?` went straight past it.
        let _ = self.release();
    }
}

/// How long a service was held still, for the manifest and the report.
fn quiesce_note(verb: &str, held: Duration) -> String {
    format!("{verb} for {:.1} s", held.as_secs_f64())
}

/// Run a short `docker compose` command, saying what it said when it failed.
fn compose_step(project: &Project, args: &[&str]) -> std::result::Result<(), String> {
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    match docker::compose_output(project, &args) {
        Ok((0, _, _)) => Ok(()),
        Ok((code, _, stderr)) => Err(format!(
            "`docker compose {}` exited {code}: {}",
            args.join(" "),
            backup::first_line(&stderr)
        )),
        Err(e) => Err(format!("`docker compose {}`: {e}", args.join(" "))),
    }
}

/// Where the archive lands, as an absolute path.
fn destination(project: &Project, out: Option<&Path>) -> Result<PathBuf> {
    let cwd = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("reading the working directory: {e}"))?;
    let name = backup::archive_name(&project_name(&project.dir), &backup::stamp(backup::now()));
    let path = backup::resolve_out_path(out, out.is_some_and(Path::is_dir), &cwd, &name);
    Ok(std::path::absolute(&path).unwrap_or(path))
}

/// The project directory's own name, which names the archive and identifies
/// the deployment in the manifest.
fn project_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "chaps".to_string())
}

/// `pg_dump -Fc` the chap-core database into the stage.
///
/// postgres has to be up: the dump goes through `docker compose exec`, which
/// needs a running container. `--no-db` is the way out when it is not.
///
/// Nothing is paused for this one: `pg_dump` reads a single transactional
/// snapshot, so the dump is consistent however busy chap-core is while it
/// runs. That is also why the database is dumped rather than its volume
/// tarred.
fn dump_database(
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
fn capture_models(
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
        docker::compose_project_name(project)
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

        // A model that runs as root has no init container to read through: it
        // needs no chown, so the overlay ships none, and its volume is reached
        // the way a component's is - mounted into a throwaway busybox. Both
        // read the same bytes; the prefixed volume name is the only thing this
        // way needs that the compose way does not.
        let root = crate::compose::overrides::is_root(&model.user);
        if root && prefix.is_none() {
            entry.skipped = Some(
                "the compose project name could not be read, so the volume cannot be \
                 named; is Docker running?"
                    .to_string(),
            );
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
        let piped = match &prefix {
            Some(prefix) if root => backup::read_volume(&format!("{prefix}_{volume}"), &dest),
            _ => docker::run_compose_piped(project, &args, None, Some(&dest)),
        };
        entry.quiesce = quiesce.release();
        let piped = piped?;
        if piped.code != 0 {
            let _ = std::fs::remove_file(&dest);
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
/// `ocs` and `s3` have no init container to read their volume through, so it
/// is mounted into a throwaway busybox container instead - the one command in
/// a backup that is a plain `docker run` rather than a compose one, because
/// there is no compose service that would do it.
fn capture_components(
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
        docker::compose_project_name(project)
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

        let member = backup::component_member(part.name);
        let dest = stage.path(&member)?;
        let mut quiesce = Quiesce::hold(project, part.service, running.has(part.service));
        let read = backup::read_volume(&volume, &dest);
        entry.quiesce = quiesce.release();
        let piped = read?;
        if piped.code != 0 {
            let _ = std::fs::remove_file(&dest);
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
fn compose_exec(service: &str, cmd: &[&str]) -> Vec<String> {
    let mut args = vec!["exec".to_string(), "-T".to_string(), service.to_string()];
    args.extend(cmd.iter().map(|s| s.to_string()));
    args
}

/// `run --rm --no-deps -T <service> <cmd..>`: a throwaway container that
/// starts nothing else, so a stopped stack stays stopped.
fn compose_run(service: &str, cmd: &[&str]) -> Vec<String> {
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

/// What went in, with sizes, and where it landed.
fn human(report: &BackupReport, out: &Out) -> String {
    let manifest = &report.manifest;
    let content = manifest.content_bytes();
    let mut text = format!(
        "{}  {}  {}\n\n",
        out.heading("backup"),
        out.value(&report.path.display().to_string()),
        out.dim(&format!(
            "({} gzipped, {} of data)",
            backup::human_size(report.size_bytes),
            backup::human_size(content)
        ))
    );

    text.push_str(&format!("{}\n", out.heading("included")));
    if manifest.files.is_empty() {
        text.push_str(&format!("  {}     {}\n", out.key("files"), out.dim("none")));
    } else {
        text.push_str(&format!(
            "  {}     {} {}\n",
            out.key("files"),
            format_args!("{} file(s):", manifest.files.len()),
            out.dim(&manifest.files.join(", "))
        ));
    }
    match &manifest.database {
        Some(db) => text.push_str(&format!(
            "  {}  {} as {} {}\n",
            out.key("database"),
            db.name,
            db.user,
            out.dim(&format!(
                "({}){}",
                backup::human_size(db.size_bytes),
                db.server_version
                    .as_deref()
                    .map(|v| format!(", PostgreSQL {v}"))
                    .unwrap_or_default()
            ))
        )),
        None => text.push_str(&format!(
            "  {}  {}\n",
            out.key("database"),
            out.dim("not included (--no-db)")
        )),
    }
    let captured: Vec<&ManifestModel> = manifest.captured_models().collect();
    if captured.is_empty() {
        text.push_str(&format!("  {}    {}\n", out.key("models"), out.dim("none")));
    } else {
        for (i, model) in captured.iter().enumerate() {
            let label = if i == 0 {
                format!("  {}  ", out.key("models"))
            } else {
                "          ".to_string()
            };
            text.push_str(&format!(
                "{label}  {}  {}  {}\n",
                model.service_id,
                model.data_dir,
                out.dim(&size_note(model.size_bytes, model.quiesce.as_deref()))
            ));
        }
    }

    let parts: Vec<&ManifestComponent> = manifest.captured_components().collect();
    if parts.is_empty() {
        text.push_str(&format!("  {}     {}\n", out.key("parts"), out.dim("none")));
    } else {
        for (i, part) in parts.iter().enumerate() {
            let label = if i == 0 {
                format!("  {}   ", out.key("parts"))
            } else {
                "          ".to_string()
            };
            text.push_str(&format!(
                "{label}  {}  {}  {}\n",
                part.name,
                part.data_dir,
                out.dim(&size_note(part.size_bytes, part.quiesce.as_deref()))
            ));
        }
    }

    let skipped: Vec<(&str, &str)> = manifest
        .models
        .iter()
        .filter_map(|m| Some((m.service_id.as_str(), m.skipped.as_deref()?)))
        .chain(
            manifest
                .components
                .iter()
                .filter_map(|c| Some((c.name.as_str(), c.skipped.as_deref()?))),
        )
        .collect();
    if !skipped.is_empty() {
        text.push_str(&format!("\n{}\n", out.heading("skipped")));
        for (name, why) in skipped {
            text.push_str(&format!("  {name}  {}\n", out.warn(why)));
        }
    }
    text
}

/// `(40.0 KB)`, and how long the service was held still when it was:
/// `(40.0 KB, paused for 1.4 s)`.
fn size_note(size_bytes: u64, quiesce: Option<&str>) -> String {
    match quiesce {
        Some(held) => format!("({}, {held})", backup::human_size(size_bytes)),
        None => format!("({})", backup::human_size(size_bytes)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{DB_MEMBER, model_member};

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
            quiesce: None,
        }
    }

    fn report_with(
        database: bool,
        models: Vec<ManifestModel>,
        components: Vec<ManifestComponent>,
    ) -> BackupReport {
        BackupReport {
            path: PathBuf::from("/backups/chaps-backup-e2e-20260923-071000.tar.gz"),
            size_bytes: 5 * 1024 * 1024,
            manifest: Manifest {
                schema_version: crate::backup::SCHEMA_VERSION,
                created_by: "chaps-cli 0.1.0".into(),
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
        }
    }

    #[test]
    fn the_human_output_lists_every_part_with_its_size() {
        let text = human(
            &report_with(
                true,
                vec![model("chapkit-ewars-model", None)],
                vec![component("ocs", None)],
            ),
            &Out::default(),
        );
        assert!(text.starts_with(
            "backup  /backups/chaps-backup-e2e-20260923-071000.tar.gz  \
             (5.0 MB gzipped, 46.0 KB of data)\n"
        ));
        assert!(text.contains("files     2 file(s): .env, compose.yml"));
        assert!(text.contains("database  chap_core as chap (2.0 KB), PostgreSQL 17.6"));
        // A running service was held still for the read, and says for how long.
        assert!(
            text.contains("models    chapkit-ewars-model  /app/data  (40.0 KB, paused for 1.4 s)")
        );
        assert!(text.contains("parts     ocs  /app/data  (4.0 KB)"));
        assert!(!text.contains("skipped"));
    }

    #[test]
    fn what_was_left_out_is_said_out_loud() {
        let text = human(
            &report_with(
                false,
                vec![model("auto-arima-chapkit", Some("no volume yet"))],
                vec![component("s3", Some("--no-components"))],
            ),
            &Out::default(),
        );
        assert!(text.contains("database  not included (--no-db)"));
        assert!(text.contains("models    none"));
        assert!(text.contains("parts     none"));
        assert!(text.contains("skipped\n  auto-arima-chapkit  no volume yet"));
        assert!(text.contains("\n  s3  --no-components"));
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
        assert_eq!(project_name(Path::new("/")), "chaps");
    }
}
