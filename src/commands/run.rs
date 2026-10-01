//! `chaps run`, `ps` and `stop`: start one model and get its URL, without a
//! deployment directory of your own.
//!
//! Inside a deployment the three work on it. Anywhere else they work on a
//! *group*: a deployment chaps keeps for this user under its data directory
//! ([`crate::paths::data_dir`]), `run/<group>/`, which the first `chaps run`
//! into it creates - no chap-core, no components, model ports on `127.0.0.1`
//! only. `--group` names one, `default` when it is left out, and each group is
//! a compose project of its own. So a tool that only wants "start this model
//! and tell me where it answers" has one code path, with or without
//! `chaps init`.

use crate::cli::{
    Cli, Command, ModelPsArgs, ModelRunArgs, ModelStopArgs, ModelsAddArgs, ModelsEnableArgs,
};
use crate::commands::Ctx;
use crate::commands::docker::wait::{self, Readiness};
use crate::commands::enable::{ModelRef, enabled_id};
use crate::compose::sync::sync;
use crate::docker;
use crate::error::{ChapError, Result};
use crate::output::Out;
use crate::project::Project;
use clap::Parser;
use serde::Serialize;
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The group a `chaps run` outside a deployment lands in without `--group`.
pub const DEFAULT_GROUP: &str = "default";

/// Where `chaps run` groups publish model ports: this machine only.
const RUN_BIND: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// Where the run family acts: a deployment the caller is inside, or a group.
struct Target {
    dir: PathBuf,
    /// `None` inside a deployment of the caller's own.
    group: Option<String>,
}

/// The directory holding every group.
pub fn groups_dir() -> PathBuf {
    crate::paths::run_groups_dir()
}

/// The groups that exist, by name, in name order.
pub fn groups() -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(groups_dir()) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .filter(|(_, dir)| Project::exists(dir))
        .collect();
    out.sort();
    out
}

/// A group's directory, refusing a name that is not one path segment of
/// lowercase letters, digits, `-` and `_`.
fn group_dir(name: &str) -> Result<PathBuf> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !ok {
        return Err(ChapError::Usage(format!(
            "`{name}` is not a group name; use lowercase letters, digits, `-` and `_`, \
             such as `--group dengue`"
        ))
        .into());
    }
    Ok(groups_dir().join(name))
}

/// The deployment the command is inside, or the group it names.
fn target(ctx: &Ctx, group: Option<&str>) -> Result<Target> {
    if let Some(dir) = Project::find_root(&ctx.project_dir) {
        if group.is_some() {
            return Err(ChapError::Usage(format!(
                "--group names a `chaps run` group, and this runs inside the deployment at {}; \
                 drop --group, or run it from outside a deployment",
                dir.display()
            ))
            .into());
        }
        return Ok(Target { dir, group: None });
    }
    let name = group.unwrap_or(DEFAULT_GROUP);
    Ok(Target {
        dir: group_dir(name)?,
        group: Some(name.to_string()),
    })
}

/// A group's deployment, created on first use.
///
/// `chaps init <dir> --only none --models none`, quietly, and then the one
/// things init has no flag for: model ports on loopback by default, and the
/// group name, which every container's labels carry.
fn ensure_default(ctx: &Ctx, dir: &Path, group: &str) -> Result<()> {
    if Project::exists(dir) {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| anyhow::anyhow!("creating {}: {e}", dir.display()))?;
    let argv = [
        "chaps",
        "init",
        &dir.to_string_lossy(),
        "--only",
        "none",
        "--models",
        "none",
    ];
    let cli = Cli::try_parse_from(argv).map_err(|e| anyhow::anyhow!("{e}"))?;
    let Command::Init(args) = cli.command else {
        unreachable!("the argv names init");
    };
    crate::commands::init::create(ctx, &args, false)?;
    let (mut project, _lock) = Project::find_locked(dir)?;
    project.state.model_bind = Some(RUN_BIND);
    project.state.group = Some(group.to_string());
    project.save()?;
    ctx.out.verbose(&format!(
        "created the chaps run deployment in {}",
        dir.display()
    ));
    Ok(())
}

/// What `chaps run` did, for `--json`.
#[derive(Debug, Serialize)]
struct RunReport {
    #[serde(flatten)]
    model: ModelRef,
    /// The group it runs in; `null` inside a deployment of the caller's own.
    group: Option<String>,
    /// The deployment the model runs in, for `chaps -C <dir> logs`.
    project_dir: PathBuf,
    /// Whether this run enabled the model, or found it enabled already.
    enabled: bool,
    /// What waiting found; `null` under `--no-wait`.
    wait: Option<Readiness>,
}

/// Enable the model if it is not, start its container, and wait for it.
pub fn run(ctx: &Ctx, args: &ModelRunArgs) -> Result<()> {
    let Target { dir, group } = target(ctx, args.group.as_deref())?;
    if let Some(group) = &group {
        ensure_default(ctx, &dir, group)?;
    }
    let ctx = &Ctx {
        project_dir: dir.clone(),
        ..ctx.clone()
    };

    let (mut project, lock) = ctx.project_mut()?;
    let registry = super::registry_for(ctx, Some(&project))?;
    let (id, enabled) = match enabled_id(&project, &args.source) {
        // Already there: started as it is, whatever port it has.
        Some(id) => (id, false),
        None => (enable_source(ctx, &mut project, args, &registry)?, true),
    };
    // A model found enabled may sit behind compose files an older chaps
    // wrote; one sync makes them current before compose reads them.
    let registry = super::registry_for(ctx, Some(&project))?;
    let synced = sync(&mut project, &registry, false)?;
    for warning in &synced.warnings {
        crate::output::warn(warning);
    }
    drop(lock);

    let model = project.state.models[&id].clone();
    let service = model.service_id.clone();
    note(
        ctx,
        &format!("starting {service} ({} in {})", id, dir.display()),
    );
    let args_up = vec![
        "up".to_string(),
        "-d".to_string(),
        "--remove-orphans".to_string(),
        service.clone(),
    ];
    let code = docker::run_compose(&project, &args_up)?;
    if code != 0 {
        return Err(ChapError::DockerFailed(code).into());
    }

    let readiness = (!args.no_wait).then(|| {
        wait::wait_until_ready(
            ctx,
            &project,
            Duration::from_secs(args.timeout),
            Some(&service),
        )
    });
    let report = RunReport {
        model: ModelRef::of(&id, &model, &project),
        group,
        project_dir: dir.clone(),
        enabled,
        wait: readiness,
    };
    if let Some(wait) = &report.wait
        && !wait.ready
    {
        let state = wait
            .models
            .first()
            .map(|m| m.state)
            .unwrap_or("not running");
        return Err(anyhow::anyhow!(
            "{service} did not answer within {}s ({state}); {}",
            args.timeout,
            logs_hint(&dir, &service)
        ));
    }
    ctx.out.emit_ok(&report, || run_summary(&report, &ctx.out))
}

/// Enable a marketplace model by id, or add whatever else the source names.
fn enable_source(
    ctx: &Ctx,
    project: &mut Project,
    args: &ModelRunArgs,
    registry: &crate::registry::Registry,
) -> Result<String> {
    if let Some(model) = registry.get(&args.source) {
        let id = model.id.clone();
        super::enable::enable_in(
            ctx,
            project,
            &ModelsEnableArgs {
                id: id.clone(),
                channel: None,
                version: None,
                port: Some(args.port),
                bind: args.bind,
                data_dir: None,
                user: None,
                allow_template: false,
            },
        )?;
        return Ok(id);
    }
    let added = super::manual_models::add_in(
        ctx,
        project,
        &ModelsAddArgs {
            source: args.source.clone(),
            id: args.id.clone(),
            service_id: None,
            name: None,
            port: Some(args.port),
            bind: args.bind,
            data_dir: None,
            user: None,
            runtime_amd64: false,
        },
    )?;
    Ok(match added {
        super::manual_models::Added::Marketplace(report) => report
            .enabled
            .iter()
            .chain(&report.updated)
            .next()
            .map(|(id, _)| id.clone())
            .expect("enabling a marketplace model reports it"),
        super::manual_models::Added::Manual(report, _) => report.id.clone(),
    })
}

/// The human rendering of a run.
fn run_summary(report: &RunReport, out: &Out) -> String {
    let url = report
        .model
        .url
        .clone()
        .unwrap_or_else(|| "internal".to_string());
    let mut text = match &report.wait {
        Some(wait) => format!(
            "{} {} on {} (answered in {}s)\n",
            out.ok("running"),
            report.model.id,
            out.value(&url),
            wait.waited_s
        ),
        None => format!(
            "{} {} on {} (not waited for; `chaps ps` says when it answers)\n",
            out.ok("started"),
            report.model.id,
            out.value(&url)
        ),
    };
    if let Some(group) = report.group.as_deref().filter(|g| *g != DEFAULT_GROUP) {
        text.push_str(&format!("{}\n", out.dim(&format!("in group {group}"))));
    }
    text.push_str(&out.backticks(&format!(
        "stop it with `chaps stop {}`; {}",
        report.model.id,
        logs_hint(&report.project_dir, &report.model.service_id)
    )));
    text
}

/// The `logs` command for a service, with `-C` when the deployment is not
/// one the working directory is inside.
fn logs_hint(dir: &Path, service: &str) -> String {
    let here = std::env::current_dir()
        .ok()
        .and_then(|cwd| Project::find_root(&cwd));
    match here.as_deref() == Some(dir) {
        true => format!("`chaps logs {service}` shows its log"),
        false => format!(
            "`chaps -C {} logs {service}` shows its log",
            shell_quote(&dir.to_string_lossy())
        ),
    }
}

/// `text`, quoted for a POSIX shell when it needs to be.
fn shell_quote(text: &str) -> String {
    match text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-~".contains(c))
    {
        true => text.to_string(),
        false => format!("'{}'", text.replace('\'', r"'\''")),
    }
}

/// One model as `chaps ps` lists it.
#[derive(Debug, Serialize)]
pub struct PsRow {
    /// The group it runs in; `null` inside a deployment of the caller's own.
    pub group: Option<String>,
    #[serde(flatten)]
    pub model: ModelRef,
    /// The `chaps status` STATE word.
    pub state: &'static str,
    pub project_dir: PathBuf,
}

/// What `chaps ps` found, for `--json`.
#[derive(Debug, Serialize)]
struct PsReport {
    models: Vec<PsRow>,
}

/// The deployments `ps`, `stop` and `top` look at: the one the caller is
/// inside, else every group (or the one `group` names).
pub(crate) fn scope(ctx: &Ctx, group: Option<&str>) -> Result<Vec<(Option<String>, PathBuf)>> {
    if let Some(dir) = Project::find_root(&ctx.project_dir) {
        target(ctx, group)?;
        return Ok(vec![(None, dir)]);
    }
    Ok(match group {
        Some(name) => {
            let dir = group_dir(name)?;
            match Project::exists(&dir) {
                true => vec![(Some(name.to_string()), dir)],
                false => Vec::new(),
            }
        }
        None => groups()
            .into_iter()
            .map(|(name, dir)| (Some(name), dir))
            .collect(),
    })
}

/// Every model in `scope`, with its state as `chaps status` would say it.
pub(crate) fn rows(scope: &[(Option<String>, PathBuf)]) -> Result<Vec<PsRow>> {
    let mut out = Vec::new();
    for (group, dir) in scope {
        let project = Project::load(dir)?;
        let running: BTreeSet<String> = docker::running_containers(&project)
            .as_deref()
            .map(docker::running_of)
            .unwrap_or_default();
        let token = crate::api::token_for(Some(&project.dir));
        let status = crate::status::status(
            &project,
            &project.api_url(),
            Duration::from_secs(3),
            &running,
            token.as_deref(),
            true,
        );
        out.extend(project.state.models.iter().map(|(id, model)| {
            PsRow {
                group: group.clone(),
                model: ModelRef::of(id, model, &project),
                state: status
                    .models
                    .iter()
                    .find(|row| row.id == model.service_id)
                    .map(|row| row.state.label())
                    .unwrap_or("not running"),
                project_dir: dir.clone(),
            }
        }));
    }
    Ok(out)
}

/// List the models with their state and URL.
pub fn ps(ctx: &Ctx, args: &ModelPsArgs) -> Result<()> {
    let scope = scope(ctx, args.group.as_deref())?;
    let report = PsReport {
        models: rows(&scope)?,
    };
    ctx.out
        .emit(&report, || ps_table(&report, &scope, &ctx.out))
}

/// The human rendering of `chaps ps`.
fn ps_table(report: &PsReport, scope: &[(Option<String>, PathBuf)], out: &Out) -> String {
    if report.models.is_empty() {
        return out.backticks(match scope.is_empty() {
            true => "nothing has been started with `chaps run` yet; `chaps run <model>` starts one",
            false => "no models are enabled here; `chaps run <model>` starts one",
        });
    }
    let grouped = report.models.iter().any(|row| row.group.is_some());
    let mut headers = vec!["ID", "SERVICE", "STATE", "URL"];
    if grouped {
        headers.insert(0, "GROUP");
    }
    let rows: Vec<Vec<String>> = report
        .models
        .iter()
        .map(|row| {
            let mut cells = vec![
                row.model.id.clone(),
                row.model.service_id.clone(),
                row.state.to_string(),
                row.model
                    .url
                    .clone()
                    .unwrap_or_else(|| "internal".to_string()),
            ];
            if grouped {
                cells.insert(0, row.group.clone().unwrap_or_default());
            }
            cells
        })
        .collect();
    let mut text = out.table(&headers, &rows);
    if grouped {
        text.push_str(&out.dim(&format!("groups live in {}", groups_dir().display())));
    }
    text
}

/// What `chaps stop` did, for `--json`.
#[derive(Debug, Serialize)]
struct StopReport {
    stopped: Vec<StoppedModel>,
}

#[derive(Debug, Serialize)]
struct StoppedModel {
    group: Option<String>,
    id: String,
    #[serde(flatten)]
    report: super::enable::DisableReport,
    notes: Vec<String>,
}

/// Stop a model, or every model in scope: its container goes and its overlay
/// with it; the data volume stays unless `--purge` says otherwise, and the
/// definition of a model that was added stays too, so `chaps run` starts it
/// again without asking GitHub.
pub fn stop(ctx: &Ctx, args: &ModelStopArgs) -> Result<()> {
    let scope = scope(ctx, args.group.as_deref())?;
    // Which deployment holds which model, settled before anything is locked.
    let mut wanted: Vec<(Option<String>, PathBuf, String)> = Vec::new();
    for (group, dir) in &scope {
        let project = Project::load(dir)?;
        match &args.id {
            Some(id) => {
                if let Some(id) = enabled_id(&project, id) {
                    wanted.push((group.clone(), dir.clone(), id));
                }
            }
            None => wanted.extend(
                project
                    .state
                    .models
                    .keys()
                    .map(|id| (group.clone(), dir.clone(), id.clone())),
            ),
        }
    }
    if let Some(id) = &args.id {
        if wanted.is_empty() {
            return Err(anyhow::anyhow!(
                "{id} is not running {}; `chaps ps` lists what is",
                match scope.as_slice() {
                    [(None, dir)] => format!("in {}", dir.display()),
                    _ => "in any `chaps run` group".to_string(),
                }
            ));
        }
        if wanted.len() > 1 {
            let names: Vec<String> = wanted
                .iter()
                .filter_map(|(group, _, _)| group.clone())
                .collect();
            return Err(ChapError::Usage(format!(
                "{id} runs in the groups {}; name one with `--group {}`",
                names.join(", "),
                names[0]
            ))
            .into());
        }
    }

    let mut stopped = Vec::new();
    for (group, dir, id) in wanted {
        let (report, notes) = stop_in(ctx, &dir, &id, args.purge)?;
        stopped.push(StoppedModel {
            group,
            id,
            report,
            notes,
        });
    }
    let report = StopReport { stopped };
    ctx.out.emit_ok(&report, || {
        if report.stopped.is_empty() {
            return ctx
                .out
                .backticks("nothing was running to stop; `chaps ps` lists what is");
        }
        let mut text = String::new();
        for model in &report.stopped {
            let place = match model.group.as_deref() {
                Some(group) if group != DEFAULT_GROUP => format!(" (group {group})"),
                _ => String::new(),
            };
            text.push_str(&format!(
                "{} {}{place}\n",
                ctx.out.warn("stopped"),
                model.id
            ));
            for note in &model.notes {
                text.push_str(&format!("{}\n", ctx.out.backticks(note)));
            }
        }
        let again = match report.stopped.as_slice() {
            [one] => format!("`chaps run {}` starts it again", one.id),
            _ => "`chaps run <model>` starts one again".to_string(),
        };
        text.push_str(&ctx.out.backticks(&again));
        text
    })
}

/// Stop one enabled model of the deployment in `dir`, printing nothing: what
/// `chaps stop` and the stop key of `chaps top` share.
pub(crate) fn stop_in(
    ctx: &Ctx,
    dir: &Path,
    id: &str,
    purge: bool,
) -> Result<(super::enable::DisableReport, Vec<String>)> {
    let ctx = &Ctx {
        project_dir: dir.to_path_buf(),
        ..ctx.clone()
    };
    let (mut project, _lock) = ctx.project_mut()?;
    let id = enabled_id(&project, id).ok_or_else(|| ChapError::UnknownModel(id.to_string()))?;
    let registry = super::registry_for(ctx, Some(&project))?;
    super::enable::disable_enabled(&mut project, &registry, &id, purge, false)
}

/// A progress line: stdout for a person, stderr under `--json`.
fn note(ctx: &Ctx, line: &str) {
    if ctx.out.json {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}
