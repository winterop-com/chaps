//! `chaps chap`: chap-core's own command line, run in a container.
//!
//! The current directory is mounted at the same path inside the container,
//! so the file arguments mean what they mean on the host and every file chap
//! writes is on the host after the run. The caches and the model runs live
//! under chaps' data directory, mounted the same way. See `docs/chap-cli.md`.

mod files;
mod plan;

use crate::cli::{ChapArgs, ChapImage};
use crate::commands::Ctx;
use crate::commands::run::target;
use crate::docker;
use crate::error::{ChapError, Result};
use crate::output;
use crate::project::Project;
use files::Snapshot;
use plan::{ModelKind, RunSpec};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

/// The exit code `docker run` gives when it could not start the container
/// at all: an image that does not pull, a mount docker refuses.
const DOCKER_RUN_FAILED: i32 = 125;

/// How many written files the closing line names.
const WRITTEN_LIMIT: usize = 8;

/// Run the chap CLI in a container, and say what it wrote.
pub fn run(ctx: &Ctx, args: &ChapArgs) -> Result<()> {
    if ctx.out.json {
        return Err(ChapError::Usage(
            "`chaps chap` prints chap's own output, so --json does not apply; drop it".to_string(),
        )
        .into());
    }
    let cwd = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("reading the current directory: {e}"))?;
    let place = Place::find(ctx, args.group.as_deref())?;

    let kind = plan::model_kind(plan::model_name(&args.args));
    let image = args
        .image
        .unwrap_or_else(|| plan::image_for(&kind, place.deployment.is_some()));
    let repository = plan::repository(image);
    let tag = resolve_tag(
        ctx,
        args.tag.as_deref(),
        place.deployment.as_ref(),
        repository,
    )?;
    let reference = format!("{repository}:{tag}");

    if let ModelKind::Chapkit(url) = &kind
        && let Some(port) = plan::loopback_port(url)
    {
        return Err(ChapError::Usage(loopback_refusal(url, port, &place)).into());
    }
    if !args.docker && runs_in_docker(ctx, &kind, &cwd) {
        return Err(ChapError::Usage(format!(
            "`{}` runs in docker (`docker_env` in its MLproject), and the container has no \
             docker socket; run it again with `chaps chap --docker ...`",
            plan::model_name(&args.args).unwrap_or_default()
        ))
        .into());
    }

    let data = crate::paths::data_dir().join("chap");
    for dir in ["runs", "cache"] {
        std::fs::create_dir_all(data.join(dir))
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", data.join(dir).display()))?;
    }
    let outside = plan::outside_paths(&args.args, &cwd);
    let mounts =
        plan::mount_dirs(&outside, &[cwd.clone(), data.clone()], Path::exists).map_err(|path| {
            ChapError::Usage(format!(
                "`{}` is outside the current directory, and no directory above it exists to \
                 mount; create its directory first, or use a path in {}",
                path.display(),
                cwd.display()
            ))
        })?;

    let socket_group = args.docker.then(|| {
        let gid = docker::socket_gid(crate::backup::BUSYBOX_IMAGE);
        if gid.is_none() {
            output::warn(
                "could not read the docker socket's group; the run may be refused the socket",
            );
        }
        gid
    });
    let spec = RunSpec {
        image: reference.clone(),
        cwd: cwd.clone(),
        mounts: mounts.clone(),
        data,
        user: host_user(),
        network: place.network.clone(),
        docker: socket_group,
        tty: std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        args: args.args.clone(),
    };

    announce(
        &args.args,
        image,
        &reference,
        &cwd,
        &mounts,
        &place,
        args.docker,
    );
    let roots: Vec<PathBuf> = std::iter::once(cwd.clone()).chain(mounts).collect();
    let before = Snapshot::take(&roots);
    let code = docker::run_plain(&plan::docker_args(&spec))?;
    let written = before.written(&Snapshot::take(&roots));

    match code {
        0 => {
            report(&written, &cwd);
            Ok(())
        }
        DOCKER_RUN_FAILED => Err(ChapError::Exited {
            command: "docker run",
            code,
            next: format!(
                "docker could not start {reference}, and its message is above; \
                 `chaps chap --tag TAG` picks another chap-core tag"
            ),
        }
        .into()),
        _ => {
            if !written.is_empty() {
                report(&written, &cwd);
            }
            Err(ChapError::Exited {
                command: "chap",
                code,
                next: "its own message is above".to_string(),
            }
            .into())
        }
    }
}

/// The deployment or group the run belongs to, when there is one, and the
/// network that reaches its models.
struct Place {
    /// The deployment the command runs in; `None` for a group or nothing.
    deployment: Option<Project>,
    /// The project whose network the container joins.
    project: Option<Project>,
    /// `<compose project>_default`, when that network exists.
    network: Option<String>,
}

impl Place {
    fn find(ctx: &Ctx, group: Option<&str>) -> Result<Place> {
        let target = target(ctx, group)?;
        let loaded = if target.dir.join(crate::project::CHAPS_DIR).is_dir() {
            Some(Project::load(&target.dir)?)
        } else if let Some(name) = group {
            return Err(ChapError::Usage(format!(
                "there is no `chaps run` group called `{name}`; `chaps ps` lists the groups, \
                 and `chaps run MODEL --group {name}` starts one"
            ))
            .into());
        } else {
            None
        };
        let network = loaded
            .as_ref()
            .and_then(Project::compose_project_name)
            .filter(|name| docker::default_network_exists(name))
            .map(|name| format!("{name}_default"));
        let deployment = loaded.clone().filter(|_| target.group.is_none());
        Ok(Place {
            deployment,
            project: loaded,
            network,
        })
    }

    /// `id at http://<service>:8000` for every model the network reaches.
    fn model_urls(&self) -> Vec<String> {
        let Some(project) = &self.project else {
            return Vec::new();
        };
        project
            .state
            .models
            .iter()
            .map(|(id, model)| format!("{id} at http://{}:{}", model.service_id, plan::MODEL_PORT))
            .collect()
    }
}

/// Why a model URL on `localhost` cannot work, and the URL that does: the
/// service name of the model published on that port, when chaps knows it.
fn loopback_refusal(url: &str, port: u16, place: &Place) -> String {
    let service = place.project.as_ref().and_then(|project| {
        project
            .state
            .models
            .values()
            .find(|model| model.host_port == Some(port))
            .map(|model| model.service_id.clone())
    });
    match (service, &place.network) {
        (Some(service), Some(_)) => format!(
            "`{url}` is this machine, and in the container `localhost` is the container \
             itself; use `--model-name http://{service}:{}`, the same model on the network",
            plan::MODEL_PORT
        ),
        _ => format!(
            "`{url}` is this machine, and in the container `localhost` is the container \
             itself; start the model with `chaps run MODEL` and use the `http://SERVICE:{}` \
             URL that `chaps chap` lists on its `models:` line",
            plan::MODEL_PORT
        ),
    }
}

/// The tag to run: `--tag`, else the deployment's own, else the newest
/// release, else the newest one this docker has.
fn resolve_tag(
    ctx: &Ctx,
    asked: Option<&str>,
    deployment: Option<&Project>,
    repository: &str,
) -> Result<String> {
    if let Some(tag) = asked {
        return Ok(tag.to_string());
    }
    if let Some(project) = deployment {
        return Ok(project.state.chap_image_tag.clone());
    }
    let local = || plan::newest_tag(&docker::local_tags(repository));
    if ctx.registry.offline {
        return local().ok_or_else(|| {
            anyhow::anyhow!(
                "--offline needs a {repository} image on this machine, and there is none; \
                 run once without --offline, or `docker pull {repository}:TAG`"
            )
        });
    }
    match crate::chapcore::latest_release(ctx.registry.timeout) {
        Ok(tag) => Ok(tag),
        Err(err) => {
            let tag = local().ok_or_else(|| {
                err.context(format!(
                    "the newest chap-core release could not be read, and this machine has no \
                     {repository} image; pass `--tag TAG`"
                ))
            })?;
            output::warn(&format!(
                "the newest chap-core release could not be read; using {tag}, the newest \
                 image on this machine"
            ));
            Ok(tag)
        }
    }
}

/// Whether the model's `MLproject` asks for `docker_env`. `false` when it
/// cannot be read: chap-core says so itself if it does, so this check only
/// moves the answer before the run.
fn runs_in_docker(ctx: &Ctx, kind: &ModelKind, cwd: &Path) -> bool {
    let text = match kind {
        ModelKind::Local(dir) => std::fs::read_to_string(cwd.join(dir).join("MLproject")).ok(),
        ModelKind::GitHub {
            owner,
            repo,
            git_ref,
        } if !ctx.registry.offline => {
            let url = format!(
                "{}/{owner}/{repo}/{}/MLproject",
                crate::chapcore::raw_base(),
                git_ref.as_deref().unwrap_or("HEAD")
            );
            crate::chapcore::get_raw(&url, ctx.registry.timeout).ok()
        }
        _ => None,
    };
    text.is_some_and(|text| plan::uses_docker_env(&text))
}

/// The user and group to run as, so the files chap writes belong to the
/// person who ran it.
#[cfg(unix)]
fn host_user() -> Option<(u32, u32)> {
    let id = |flag: &str| -> Option<u32> {
        let out = std::process::Command::new("id").arg(flag).output().ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    };
    Some((id("-u")?, id("-g")?))
}

#[cfg(not(unix))]
fn host_user() -> Option<(u32, u32)> {
    None
}

/// The lines before the run: the image, the directories chap sees, and the
/// models it can reach.
fn announce(
    args: &[String],
    image: ChapImage,
    reference: &str,
    cwd: &Path,
    mounts: &[PathBuf],
    place: &Place,
    socket: bool,
) {
    let command = args.first().map(String::as_str).unwrap_or("--help");
    let pull = match docker::image_is_local(reference) {
        true => String::new(),
        false => format!(
            "; the first run pulls it, about {}",
            plan::approximate_size(image)
        ),
    };
    output::notice(&format!("running `chap {command}` in {reference}{pull}"));
    let mut seen: Vec<String> = vec![cwd.display().to_string()];
    seen.extend(mounts.iter().map(|dir| dir.display().to_string()));
    output::notice(&format!(
        "files: chap reads and writes in {}",
        seen.join(", ")
    ));
    match (&place.project, &place.network) {
        (Some(_), Some(_)) => {
            let urls = place.model_urls();
            if !urls.is_empty() {
                output::notice(&format!("models: {}", urls.join(", ")));
            }
        }
        (Some(project), None) => output::notice(&format!(
            "models: {} is not running, so its models are not reachable; `chaps -C {} up` \
             starts it",
            project.dir.display(),
            project.dir.display()
        )),
        _ => {}
    }
    if socket {
        output::warn("--docker gives the container control of docker on this machine");
    }
}

/// The closing line: what the run wrote, and what to do with it.
fn report(written: &[PathBuf], cwd: &Path) {
    if written.is_empty() {
        output::notice(&format!(
            "chap finished; it wrote no file in {}",
            cwd.display()
        ));
        return;
    }
    output::notice(&format!(
        "chap finished; it wrote {}",
        files::describe(written, cwd, WRITTEN_LIMIT)
    ));
    let evaluation = written
        .iter()
        .find(|path| path.extension().is_some_and(|ext| ext == "nc"));
    if let Some(nc) = evaluation {
        let name = nc.strip_prefix(cwd).unwrap_or(nc).to_string_lossy();
        let html = Path::new(name.as_ref()).with_extension("html");
        output::notice(&format!(
            "`chaps chap plot-backtest {name} --output-file {}` plots it",
            html.display()
        ));
    }
}

#[cfg(test)]
mod tests;
