//! What the images on this machine are: the build a container runs, the
//! digest it was pulled at, and how an image says it runs.

use super::{Container, RunFn, docker_capture, docker_run, trace_command, verbose_exit};
use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Stdio};

/// What one running container is actually made of.
///
/// The pair that decides whether it is stale: the id of the image it runs
/// (not the reference, which can point somewhere else after a pull) and the
/// configuration hash it was created with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainerBuild {
    pub image_id: String,
    pub config_hash: String,
}

/// What each of these containers is made of, keyed by the id it was asked
/// about (`docker inspect`).
///
/// One call for the whole list, matched back by id rather than by position,
/// so a container that vanished between the `ps` and the `inspect` costs its
/// own row and not the rows after it.
pub fn container_builds(ids: &[String]) -> BTreeMap<String, ContainerBuild> {
    if ids.is_empty() {
        return BTreeMap::new();
    }
    let mut args = vec![
        "inspect".to_string(),
        "--format".to_string(),
        INSPECT_FORMAT.to_string(),
    ];
    args.extend(ids.iter().cloned());
    let Some(text) = docker_capture(&args) else {
        return BTreeMap::new();
    };
    let found = parse_container_builds(&text);
    // `ps` prints short ids and `inspect` prints full ones, so the answer is
    // keyed back to the id the caller knows.
    ids.iter()
        .filter_map(|id| {
            let build = found
                .iter()
                .find(|(full, _)| full == id || full.starts_with(id.as_str()))?;
            Some((id.clone(), build.1.clone()))
        })
        .collect()
}

/// The `--format` [`container_builds`] asks for: full id, image id and
/// configuration hash, one tab-separated line per container.
///
/// `{{json .Config.Labels}}` rather than `{{index .Config.Labels "..."}}`
/// because the quotes that second spelling needs are one more thing to get
/// right on a Windows command line, and the whole label set costs nothing.
const INSPECT_FORMAT: &str = "{{.Id}}\t{{.Image}}\t{{json .Config.Labels}}";

/// The label compose stamps a container with, so a later run can tell whether
/// the files it was made from have changed.
const CONFIG_HASH_LABEL: &str = "com.docker.compose.config-hash";

/// Parse the [`INSPECT_FORMAT`] lines into `(full id, build)` pairs.
pub fn parse_container_builds(text: &str) -> Vec<(String, ContainerBuild)> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.trim_end().splitn(3, '\t');
            let id = fields.next()?.trim();
            let image_id = fields.next()?.trim();
            if id.is_empty() {
                return None;
            }
            let labels = fields.next().unwrap_or_default();
            let config_hash = serde_json::from_str::<serde_json::Value>(labels)
                .ok()
                .and_then(|v| {
                    v.get(CONFIG_HASH_LABEL)
                        .and_then(|h| h.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            Some((
                id.to_string(),
                ContainerBuild {
                    image_id: image_id.to_string(),
                    config_hash,
                },
            ))
        })
        .collect()
}

/// The local image id each of these references points at right now
/// (`docker image inspect`), leaving out the ones this machine does not have.
///
/// A reference is asked about once however many services share it, and a
/// reference docker has never pulled is simply absent: "we do not know" and
/// "it changed" must not be the same answer.
pub fn image_ids(refs: &[String]) -> BTreeMap<String, String> {
    let unique: BTreeSet<&String> = refs.iter().filter(|r| !r.is_empty()).collect();
    unique
        .into_iter()
        .filter_map(|reference| Some((reference.clone(), image_id(reference)?)))
        .collect()
}

/// Which build one running service is actually on, in short form.
///
/// A moving tag such as `dev` says nothing about what it is: two machines on
/// the same tag can be on different images, and so can the same machine after
/// a pull. The digest the image was pulled at is the one thing that does say,
/// so it is what `varde status`, `varde doctor` and
/// `varde update --list-tags` print next to a moving tag.
///
/// `None` all the way down when docker cannot be asked or the service is not
/// up: not knowing which build is running is reported as not knowing.
pub fn running_build(containers: &[Container], service: &str) -> Option<String> {
    let container = containers
        .iter()
        .find(|c| c.service == service && c.is_running())?;
    let build = container_builds(std::slice::from_ref(&container.id));
    let id = build.get(&container.id).map(|b| b.image_id.clone())?;
    Some(short_digest(&image_digest(&id).unwrap_or(id)))
}

/// The digest an image on this machine was pulled at, from its `RepoDigests`.
///
/// An image that was built here rather than pulled carries none, which is
/// `None`: its id is the only thing that identifies it, and the caller falls
/// back to that.
pub fn image_digest(reference: &str) -> Option<String> {
    let args = vec![
        "image".to_string(),
        "inspect".to_string(),
        "--format".to_string(),
        "{{json .RepoDigests}}".to_string(),
        reference.to_string(),
    ];
    parse_repo_digest(&docker_capture(&args)?)
}

/// The first digest of a `RepoDigests` array, as `sha256:...`.
pub fn parse_repo_digest(text: &str) -> Option<String> {
    let list: Vec<String> = serde_json::from_str(text.trim()).ok()?;
    list.iter()
        .find_map(|entry| entry.split_once('@').map(|(_, digest)| digest.to_string()))
}

/// How long a digest or image id a person is asked to compare by eye.
const SHORT_DIGEST: usize = 12;

/// A digest or image id as the twelve hex characters that identify it.
pub fn short_digest(value: &str) -> String {
    let hex = value
        .trim()
        .rsplit_once(':')
        .map(|(_, hex)| hex)
        .unwrap_or_else(|| value.trim());
    hex.chars().take(SHORT_DIGEST).collect()
}

/// The local image id one reference points at, or `None` when this machine
/// does not have it.
fn image_id(reference: &str) -> Option<String> {
    let args = vec![
        "image".to_string(),
        "inspect".to_string(),
        "--format".to_string(),
        "{{.Id}}".to_string(),
        reference.to_string(),
    ];
    let id = docker_capture(&args)?.trim().to_string();
    (!id.is_empty()).then_some(id)
}

/// The `--format` [`image_config`] asks for: the two fields the overlay is
/// rendered from, followed by the two that say whether a config was answered
/// at all.
///
/// An image store that does not hold the platform being asked about answers
/// with an empty config rather than an error, and an empty `User` reads as
/// root - so `Entrypoint` and `Cmd` are fetched as the evidence that the
/// blank `User` is the image's own. `{{json}}` for those two: they are
/// arrays, and `null` is how a config that has neither prints.
const IMAGE_CONFIG_FORMAT: &str = "{{.Config.User}}\t{{.Config.WorkingDir}}\t\
     {{json .Config.Entrypoint}}\t{{json .Config.Cmd}}";

/// What one image on this machine says about how it runs: `(user, working
/// directory)`, or `None` when this machine's image store cannot answer for
/// it as `linux/amd64`.
///
/// The same two fields the registry's config blob carries, which is what
/// makes this the fallback for `varde models add` when ghcr cannot be
/// reached: a tab-separated `docker image inspect`, so an empty field stays
/// an empty field.
///
/// The platform is pinned to [`crate::compose::AMD64_PLATFORM`], the one
/// every overlay runs the model images as. Without it, a daemon on the
/// containerd image store answers for the host's own architecture, and an
/// arm64 host that has pulled only the amd64 variant is handed an empty
/// config - which used to read as "runs as root". A docker that will not take
/// the flag at all is asked again without it, because that answer is still
/// better than none: [`is_flag_unsupported`].
pub fn image_config(reference: &str) -> Option<(String, String)> {
    image_config_with(reference, &docker_run)
}

/// [`image_config`] with the docker run injected, so the platform pin and its
/// fallback can be tested without a daemon.
pub fn image_config_with(reference: &str, run: RunFn) -> Option<(String, String)> {
    let mut args = vec![
        "image".to_string(),
        "inspect".to_string(),
        "--platform".to_string(),
        crate::compose::AMD64_PLATFORM.to_string(),
        "--format".to_string(),
        IMAGE_CONFIG_FORMAT.to_string(),
        reference.to_string(),
    ];
    let mut run_out = run(&args)?;
    if !run_out.ok && is_flag_unsupported(&run_out.stderr) {
        crate::output::verbose(
            "  this docker has no `--platform` for `image inspect`; asking without it",
        );
        args.drain(2..4);
        run_out = run(&args)?;
    }
    if !run_out.ok {
        verbose_exit(&args, run_out.code);
        return None;
    }
    parse_image_config(&run_out.stdout)
}

/// Parse the [`IMAGE_CONFIG_FORMAT`] line into `(user, working_dir)`, or
/// `None` when the whole config came back blank.
///
/// A config with no `User`, no `WorkingDir`, no `Entrypoint` and no `Cmd` is
/// not an image that runs as root in `/`: it is an image store answering
/// about a platform it does not hold. A real root image has a working
/// directory or a command, so "empty throughout" is the one safe reading of
/// "we do not have it".
pub fn parse_image_config(text: &str) -> Option<(String, String)> {
    let line = text.lines().next()?;
    let mut fields = line.split('\t');
    let user = fields.next()?.trim();
    let working_dir = fields.next()?.trim();
    let rest_blank = fields.all(is_blank_field);
    if user.is_empty() && working_dir.is_empty() && rest_blank {
        return None;
    }
    Some((user.to_string(), working_dir.to_string()))
}

/// Whether one `{{json}}` field carries nothing: what docker prints for a
/// config field it has no value for.
fn is_blank_field(field: &str) -> bool {
    matches!(field.trim(), "" | "null" | "[]" | "{}")
}

/// Whether docker refused the command over the flag rather than the image: a
/// CLI too old to know it (`image inspect` learnt `--platform` in Docker 25)
/// or one talking to a daemon whose API is older than the flag needs.
///
/// Either way the answer without the flag is the one this module gave before
/// it pinned anything, which is better than no answer. An image that is
/// simply not held for that platform is a different refusal, and it is a real
/// answer: `None`.
fn is_flag_unsupported(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("unknown flag")
        || lower.contains("unknown shorthand flag")
        || lower.contains("requires api version")
}

/// Pull one image, showing docker's own progress, and say whether it worked.
///
/// The progress goes to stderr, stdout included: it is narration, and stdout
/// belongs to the command's answer, which under `--json` has to parse.
///
/// The platform is pinned for the same reason every model overlay pins it:
/// the model images are published for amd64 only, and an Apple Silicon host
/// asked for its own architecture is told there is no matching manifest.
pub fn pull_image(reference: &str) -> bool {
    let args = [
        "pull".to_string(),
        "--platform".to_string(),
        crate::compose::AMD64_PLATFORM.to_string(),
        reference.to_string(),
    ];
    trace_command(&args);
    Command::new("docker")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(std::io::stderr())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// `config.Entrypoint` then `config.Cmd` of a local image, for the
/// [`crate::compose::resolve`] fallback when the registry cannot be asked.
/// `None` when this docker does not have the amd64 variant.
pub fn image_command(reference: &str) -> Option<Vec<String>> {
    let args = [
        "image",
        "inspect",
        "--platform",
        crate::compose::AMD64_PLATFORM,
        "--format",
        "{{json .Config.Entrypoint}}\t{{json .Config.Cmd}}",
        reference,
    ]
    .map(str::to_string);
    parse_command(&docker_capture(&args)?)
}

/// The two JSON arrays of an `image inspect` command line, joined; `null`
/// is an empty part.
pub fn parse_command(text: &str) -> Option<Vec<String>> {
    let line = text.lines().next()?;
    let mut command = Vec::new();
    for part in line.split('\t') {
        let words: Option<Vec<String>> = serde_json::from_str(part.trim()).ok()?;
        command.extend(words.unwrap_or_default());
    }
    Some(command)
}

/// The numeric `uid:gid` of an account name inside an image, by asking the
/// image itself.
///
/// The last resort of `varde models add`: an image that runs as a name this
/// CLI has never heard of still has to be chowned to something, and only the
/// image knows what that name resolves to. `sh` and `id` are in every
/// chapkit base; an image with neither yields `None` and the caller falls
/// back to the name plus a `--user` hint.
pub fn uid_gid_in_image(reference: &str, name: &str) -> Option<(u32, u32)> {
    let args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--platform".to_string(),
        crate::compose::AMD64_PLATFORM.to_string(),
        "--entrypoint".to_string(),
        "sh".to_string(),
        reference.to_string(),
        "-c".to_string(),
        format!("id -u {name}; id -g {name}"),
    ];
    let text = docker_capture(&args)?;
    parse_uid_gid(&text)
}

/// The two numbers `id -u NAME; id -g NAME` prints.
pub fn parse_uid_gid(text: &str) -> Option<(u32, u32)> {
    let mut numbers = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| line.parse::<u32>().ok());
    Some((numbers.next()?, numbers.next()?))
}
