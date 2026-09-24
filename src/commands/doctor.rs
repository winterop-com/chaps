//! `chaps doctor` - one bounded checklist that says whether this machine, and
//! this deployment, can run CHAP.
//!
//! Every other command answers one question. `doctor` answers the ones an
//! operator would otherwise ask in sequence: is Docker there, is the daemon
//! up, is Compose new enough, is there disk, can this machine reach the hosts
//! CHAP pulls from - and, inside a deployment, are the files, the ports, the
//! pins and the running stack what they should be.
//!
//! Two rules shape the module. Every check is bounded: external commands are
//! killed when they overrun and every network probe carries a timeout, so
//! `chaps doctor` always finishes. And every verdict is a pure function of
//! data someone else gathered, so the half that decides what to say is
//! testable without Docker, a network or a project.

use crate::auth;
use crate::chapcore;
use crate::cli::DoctorArgs;
use crate::commands::Ctx;
use crate::components::{
    COMPONENTS_FILE, Component, Components, OCS_CONFIG_FILE, OCS_DIR, OCS_IMAGE, OCS_PLUGINS_DIR,
    OCS_TAG_ENV_VAR, S3_DEFAULT_TAG, S3_IMAGE, S3_TAG_ENV_VAR,
};
use crate::compose::render::OCS_EXAMPLE_MARKER;
use crate::compose::{API_SERVICE, sync};
use crate::docker;
use crate::error::Result;
use crate::output::Out;
use crate::ports::{self, PortClaim};
use crate::project::{
    ApiPortSource, BASE_COMPOSE, CHAP_TAG_ENV_VAR, CHAPS_COMPOSE, CHAPS_DIR, ENV_FILE,
    MARKETPLACE_COMPOSE, MODELS_FILE, PROJECT_FILE, Project,
};
use crate::registry;
use crate::selfupdate::{self, TARGET, VERSION};
use crate::status::{ApiHealth, ComponentState, StatusReport};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long one network probe may take. Three seconds is long enough for a
/// slow link and short enough that four of them in parallel still leave the
/// whole command inside a few seconds.
pub const NET_TIMEOUT: Duration = Duration::from_secs(3);

/// How long the `docker` CLI may take to answer one question.
pub const DOCKER_TIMEOUT: Duration = Duration::from_secs(5);

/// How long one `docker manifest inspect` may take. Longer than a probe: it
/// is a registry round trip with an authentication handshake in front of it.
pub const MANIFEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the `stack` check waits on chap-core's API.
pub const STACK_TIMEOUT: Duration = Duration::from_secs(5);

/// How often a bounded child is asked whether it has finished.
const POLL: Duration = Duration::from_millis(25);

/// One gibibyte, the unit the disk thresholds are written in.
pub const GB: u64 = 1024 * 1024 * 1024;

/// Free space below which `disk` warns: the images total several GB and a
/// pull that runs out halfway leaves a half-populated cache behind.
pub const DISK_WARN: u64 = 10 * GB;

/// Free space below which `disk` fails outright.
pub const DISK_FAIL: u64 = 3 * GB;

/// Columns the status word is padded to (`warn`, `fail` and `skip` are the
/// widest).
pub const STATUS_WIDTH: usize = 4;

/// Spaces between the columns of one line, and the width of the indent a fix
/// line is written at.
const GAP: usize = 2;

/// The container registry every CHAP image is pulled from. The anonymous
/// `/v2/` endpoint answers 401, which is still the host answering.
pub const GHCR_PROBE_URL: &str = "https://ghcr.io/v2/";

/// `User-Agent` sent with the probes, matching the registry fetch.
const USER_AGENT: &str = concat!("chaps-cli/", env!("CARGO_PKG_VERSION"));

/// The one `docker info` the checklist runs, asked for both fields it needs:
/// the engine version for the `docker-daemon` line and the data root for
/// `disk`. Two calls would be two round trips to the same daemon.
const INFO_FORMAT: &str = "{{.ServerVersion}}\t{{.DockerRootDir}}";

/// What `doctor` says instead of the project section when it was not run
/// inside a deployment directory.
pub const NO_PROJECT: &str =
    "project: none here (run chaps doctor inside a deployment directory for more)";

/// What one check concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Nothing to do.
    Ok,
    /// It works, but something will bite later.
    Warn,
    /// CHAP will not work until this is dealt with.
    Fail,
    /// Not evaluated: there was nothing to ask, or asking was ruled out.
    Skip,
}

impl Status {
    /// The word printed in the first column.
    pub fn word(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
            Status::Skip => "skip",
        }
    }

    /// The word in the colour its meaning has everywhere else in the CLI.
    fn paint(self, out: &Out) -> String {
        match self {
            Status::Ok => out.ok(self.word()),
            Status::Warn => out.warn(self.word()),
            Status::Fail => out.bad(self.word()),
            Status::Skip => out.dim(self.word()),
        }
    }
}

/// One line of the checklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// Stable machine name, for a script that greps one line out of `--json`.
    pub id: String,
    /// What is printed in the second column.
    pub name: String,
    pub status: Status,
    /// The short result: what was found, in as few words as say it.
    pub detail: String,
    /// What to do about it, printed on an indented line of its own. `None`
    /// when there is nothing to do.
    pub fix: Option<String>,
}

impl Check {
    fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        status: Status,
        detail: impl Into<String>,
        fix: Option<String>,
    ) -> Check {
        Check {
            id: id.into(),
            name: name.into(),
            status,
            detail: detail.into(),
            fix,
        }
    }

    /// A check that passed. Nothing passed needs a fix.
    pub fn ok(id: impl Into<String>, name: impl Into<String>, detail: impl Into<String>) -> Check {
        Check::new(id, name, Status::Ok, detail, None)
    }

    pub fn warn(
        id: impl Into<String>,
        name: impl Into<String>,
        detail: impl Into<String>,
        fix: impl Into<String>,
    ) -> Check {
        Check::new(id, name, Status::Warn, detail, Some(fix.into()))
    }

    pub fn fail(
        id: impl Into<String>,
        name: impl Into<String>,
        detail: impl Into<String>,
        fix: impl Into<String>,
    ) -> Check {
        Check::new(id, name, Status::Fail, detail, Some(fix.into()))
    }

    /// A check that was not evaluated, and why.
    pub fn skip(
        id: impl Into<String>,
        name: impl Into<String>,
        detail: impl Into<String>,
    ) -> Check {
        Check::new(id, name, Status::Skip, detail, None)
    }

    /// [`Check::skip`] that still has something worth typing next to it.
    pub fn skip_with(
        id: impl Into<String>,
        name: impl Into<String>,
        detail: impl Into<String>,
        fix: impl Into<String>,
    ) -> Check {
        Check::new(id, name, Status::Skip, detail, Some(fix.into()))
    }

    /// Build one from a verdict: the shape the pure decision functions return.
    fn from_verdict(
        id: impl Into<String>,
        name: impl Into<String>,
        verdict: (Status, String, Option<String>),
    ) -> Check {
        Check::new(id, name, verdict.0, verdict.1, verdict.2)
    }
}

/// How many checks landed in each status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub ok: usize,
    pub warn: usize,
    pub fail: usize,
    pub skip: usize,
}

/// What `chaps doctor` prints, and the whole of its `--json`.
#[derive(Debug, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
    pub summary: Summary,
    /// The deployment the project half of the checklist was run against.
    /// Not serialised: `--json` is the two documented keys and nothing else,
    /// and a consumer reads the absence of the project checks instead.
    #[serde(skip)]
    pub project: Option<PathBuf>,
}

/// Count the statuses of a finished checklist.
pub fn summary(checks: &[Check]) -> Summary {
    let mut summary = Summary::default();
    for check in checks {
        match check.status {
            Status::Ok => summary.ok += 1,
            Status::Warn => summary.warn += 1,
            Status::Fail => summary.fail += 1,
            Status::Skip => summary.skip += 1,
        }
    }
    summary
}

/// The process exit code: non-zero only when something failed.
///
/// A warning is a thing to know about, not a thing that stops CHAP, so
/// `chaps doctor` in a CI step only goes red on the checks that would have
/// stopped the deployment anyway.
pub fn exit_code(summary: &Summary) -> i32 {
    i32::from(summary.fail > 0)
}

/// The closing line: how many checks ran and how they came out.
pub fn summary_line(summary: &Summary) -> String {
    let total = summary.ok + summary.warn + summary.fail + summary.skip;
    let mut line = format!(
        "{total} checks: {} ok, {} warn, {} fail",
        summary.ok, summary.warn, summary.fail
    );
    // Skipped checks are left out of the sentence when there are none, so the
    // common case reads as the three words that matter.
    if summary.skip > 0 {
        line.push_str(&format!(", {} skipped", summary.skip));
    }
    line
}

/// The human rendering: one line per check, an indented fix under the ones
/// that have one, and the summary at the bottom.
///
/// The name column is padded to the widest name so the results line up, and
/// the padding is written outside the styled spans so a coloured word is the
/// same width as a plain one.
pub fn render(report: &Report, out: &Out) -> String {
    let name_width = report
        .checks
        .iter()
        .map(|check| check.name.chars().count())
        .max()
        .unwrap_or(0);
    let indent = " ".repeat(STATUS_WIDTH + GAP);
    let mut text = String::new();
    for check in &report.checks {
        text.push_str(&check.status.paint(out));
        text.push_str(&" ".repeat(STATUS_WIDTH - check.status.word().len() + GAP));
        text.push_str(&check.name);
        text.push_str(&" ".repeat(name_width - check.name.chars().count() + GAP));
        text.push_str(check.detail.trim());
        // A check with nothing to report must not leave a line of spaces.
        while text.ends_with(' ') {
            text.pop();
        }
        text.push('\n');
        if let Some(fix) = &check.fix {
            text.push_str(&indent);
            text.push_str(&out.backticks(fix.trim()));
            text.push('\n');
        }
    }
    if report.project.is_none() {
        text.push_str(&out.dim(NO_PROJECT));
        text.push('\n');
    }
    text.push('\n');
    text.push_str(&out.cmd(&summary_line(&report.summary)));
    text.push('\n');
    text
}

/// Run the checklist and print it.
pub fn run(ctx: &Ctx, _args: &DoctorArgs) -> Result<()> {
    // Not `ctx.project()`: outside a deployment `doctor` still has most of a
    // report to give, so a missing project is a section that is left out
    // rather than an error.
    let project = Project::find(&ctx.project_dir).ok();
    let checks = collect(ctx, project.as_ref());
    let report = Report {
        summary: summary(&checks),
        checks,
        project: project.map(|project| project.dir),
    };
    ctx.out.emit(&report, || render(&report, &ctx.out))?;
    let code = exit_code(&report.summary);
    if code != 0 {
        // Everything there was to say is on the screen already; an `error:`
        // line on top of a checklist that names each failure would repeat it.
        std::process::exit(code);
    }
    Ok(())
}

/// Every check, in the order they are printed.
fn collect(ctx: &Ctx, project: Option<&Project>) -> Vec<Check> {
    let offline = ctx.registry.offline;
    let registry_url = ctx.registry.url.clone();

    std::thread::scope(|scope| {
        // Started first and joined last: the network is the slow half, and
        // the docker questions below are answered while it is in flight.
        let probes = (!offline).then(|| Probes::spawn(scope, &registry_url));

        let mut checks = Vec::new();

        let client = run_bounded(
            "docker",
            &["version", "--format", "{{.Client.Version}}"],
            DOCKER_TIMEOUT,
        );
        let have_cli = client.succeeded();
        checks.push(docker_cli_check(&client));

        let info = if have_cli {
            run_bounded("docker", &["info", "--format", INFO_FORMAT], DOCKER_TIMEOUT)
        } else {
            Outcome::Missing
        };
        let root_dir = info_fields(info.stdout()).1;
        checks.push(docker_daemon_check(&info, have_cli));

        // Nothing to ask compose when there is no CLI to ask it through.
        let compose = have_cli.then(|| docker::compose_version().ok()).flatten();
        checks.push(compose_check(have_cli, compose));
        checks.push(arch_check(std::env::consts::OS, std::env::consts::ARCH));

        let measured = disk_path(root_dir.as_deref());
        checks.push(disk_check(&measured, free_bytes(&measured)));

        let probed = probes.map(Probes::join);
        checks.extend(network_checks(probed.as_ref()));
        checks.push(chaps_check(ReleaseList::of(
            probed.as_ref().map(|p| &p.chaps),
        )));

        if let Some(project) = project {
            checks.extend(project_checks(ctx, project, probed.as_ref(), have_cli));
        }
        checks
    })
}

/// The half of the checklist that needs a deployment directory.
fn project_checks(
    ctx: &Ctx,
    project: &Project,
    probed: Option<&Probed>,
    have_cli: bool,
) -> Vec<Check> {
    // The containers are asked for once, before the checks that need them:
    // the ports they hold are not conflicts, whether any of them is up
    // decides the `stack` check, and which of them is up decides where the
    // `components` and `volumes` lines read what OCS holds - from inside the
    // container, or from its volume.
    let containers = docker::all_containers(project);
    let running: BTreeSet<String> = containers
        .as_deref()
        .map(docker::running_of)
        .unwrap_or_default();

    let mut checks = vec![
        project_check(project),
        files_check(&project.dir, &project.state.components),
        components_check(project, &running),
        sync_check(ctx, project),
        env_check(
            std::fs::read_to_string(project.dir.join(ENV_FILE))
                .ok()
                .as_deref(),
            std::fs::read_to_string(project.dir.join(CHAPS_COMPOSE))
                .ok()
                .as_deref(),
        ),
        volumes_check(project, &running),
    ];
    checks.extend(manual_checks(ctx, project));

    let claims = ports::claims(project);
    let busy = ports::busy_claims(&claims, &running, &ports::is_busy);
    // A port to point the API at, found the way `chaps up` finds one.
    let suggestion = busy
        .iter()
        .find(|claim| claim.service == API_SERVICE)
        .and_then(|claim| {
            ports::first_free(claim.port.saturating_add(1), u16::MAX, &ports::is_busy)
        });
    // Only the API port has two files that could have set it, so only it ever
    // has a note.
    let api_note = api_port_note(project);
    for claim in &claims {
        let note = (claim.service == API_SERVICE)
            .then_some(api_note.as_deref())
            .flatten();
        checks.push(port_check(
            claim,
            &running,
            &ports::is_busy,
            suggestion,
            note,
        ));
    }

    // The chap-core release feed says nothing about a deployment that does not
    // run chap-core.
    if project.state.components.chap_core.enabled {
        checks.push(pin_check(
            &project.state.chap_image_tag,
            ReleaseList::of(probed.map(|p| &p.chap_core)),
        ));
    }
    checks.extend(image_checks(project, probed, have_cli));
    checks.extend(user_checks(project, have_cli));
    checks.push(stack_check(project, containers.as_deref(), &running));
    checks
}

// ---------------------------------------------------------------------------
// Machine checks
// ---------------------------------------------------------------------------

/// Whether `docker` is on PATH and answers at all.
pub fn docker_cli_check(outcome: &Outcome) -> Check {
    const NAME: &str = "docker cli";
    const FIX: &str =
        "install Docker Desktop or the docker CLI: https://docs.docker.com/get-docker/";
    match outcome {
        Outcome::Done {
            ok: true, stdout, ..
        } => Check::ok("docker-cli", NAME, format!("docker {}", first_line(stdout))),
        Outcome::Missing => Check::fail("docker-cli", NAME, "`docker` is not on PATH", FIX),
        Outcome::TimedOut => Check::fail(
            "docker-cli",
            NAME,
            format!(
                "`docker version` did not answer within {}s",
                DOCKER_TIMEOUT.as_secs()
            ),
            "check whether the docker CLI is wedged, and reinstall it if it is",
        ),
        Outcome::Failed(why) => Check::fail("docker-cli", NAME, why.clone(), FIX),
        Outcome::Done { stderr, .. } => Check::fail("docker-cli", NAME, first_line(stderr), FIX),
    }
}

/// Why `docker info` could not reach a daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonProblem {
    /// Nothing is listening on the socket.
    NotRunning,
    /// Something is, and this user may not talk to it.
    PermissionDenied,
    /// Docker said something else entirely.
    Unknown,
}

/// Classify what `docker info` printed on stderr.
///
/// The two cases worth telling apart have very different fixes, and docker
/// spells both of them out: "permission denied while trying to connect" for a
/// socket this user may not open, and "Cannot connect to the Docker daemon"
/// (plus the Windows spelling of the same thing) for one that is not there.
pub fn daemon_problem(stderr: &str) -> DaemonProblem {
    let text = stderr.to_ascii_lowercase();
    if text.contains("permission denied") {
        return DaemonProblem::PermissionDenied;
    }
    if text.contains("cannot connect to the docker daemon")
        || text.contains("is the docker daemon running")
        || text.contains("error during connect")
        || text.contains("the system cannot find the file specified")
    {
        return DaemonProblem::NotRunning;
    }
    DaemonProblem::Unknown
}

/// What to do about each of them.
pub fn daemon_fix(problem: DaemonProblem) -> &'static str {
    match problem {
        DaemonProblem::NotRunning => {
            "start Docker: open Docker Desktop, or `sudo systemctl start docker` on Linux"
        }
        DaemonProblem::PermissionDenied => {
            "add yourself to the `docker` group (`sudo usermod -aG docker $USER`, then log in \
             again), or run docker under sudo"
        }
        DaemonProblem::Unknown => "run `docker info` to see the whole message",
    }
}

/// Whether a daemon is running and this user can talk to it.
pub fn docker_daemon_check(outcome: &Outcome, have_cli: bool) -> Check {
    const NAME: &str = "docker daemon";
    if !have_cli {
        return Check::skip("docker-daemon", NAME, "no docker CLI to ask");
    }
    match outcome {
        Outcome::Done {
            ok: true, stdout, ..
        } => Check::ok(
            "docker-daemon",
            NAME,
            format!("Docker Engine {}", info_fields(stdout).0),
        ),
        Outcome::TimedOut => Check::fail(
            "docker-daemon",
            NAME,
            format!(
                "`docker info` did not answer within {}s",
                DOCKER_TIMEOUT.as_secs()
            ),
            daemon_fix(DaemonProblem::NotRunning),
        ),
        other => {
            let stderr = other.stderr();
            let problem = daemon_problem(stderr);
            let detail = match problem {
                DaemonProblem::NotRunning => "the Docker daemon is not running".to_string(),
                DaemonProblem::PermissionDenied => {
                    "permission denied on the Docker socket".to_string()
                }
                DaemonProblem::Unknown => first_line(stderr),
            };
            Check::fail("docker-daemon", NAME, detail, daemon_fix(problem))
        }
    }
}

/// Whether the installed Compose understands what `chaps` renders.
pub fn compose_verdict(found: Option<(u32, u32, u32)>) -> (Status, String, Option<String>) {
    let (wa, wb, wc) = docker::MIN_COMPOSE_VERSION;
    let Some((a, b, c)) = found else {
        return (
            Status::Fail,
            "docker compose is not installed".to_string(),
            Some(format!(
                "install the Compose v2 plugin, {wa}.{wb}.{wc} or newer: \
                 https://docs.docker.com/compose/install/"
            )),
        );
    };
    if (a, b, c) < docker::MIN_COMPOSE_VERSION {
        return (
            Status::Warn,
            format!("v{a}.{b}.{c}, older than {wa}.{wb}.{wc}"),
            Some(format!(
                "upgrade Docker Compose: compose.chaps.yml uses `!override`, which needs \
                 {wa}.{wb}.{wc} or newer, and compose.marketplace.yml uses `include:`"
            )),
        );
    }
    (Status::Ok, format!("v{a}.{b}.{c}"), None)
}

/// The `compose` line, which has nothing to say when there is no CLI to ask.
pub fn compose_check(have_cli: bool, found: Option<(u32, u32, u32)>) -> Check {
    if !have_cli {
        return Check::skip("compose", "docker compose", "no docker CLI to ask");
    }
    Check::from_verdict("compose", "docker compose", compose_verdict(found))
}

/// Whether this host runs the published images natively.
///
/// Not a probe: whether emulation is configured can only be learned by
/// running an amd64 container, which is far too slow for a checklist. The
/// line says what the host is and what that implies, and leaves the reader to
/// act on it.
pub fn arch_check(os: &str, arch: &str) -> Check {
    const ID: &str = "arch";
    const NAME: &str = "os and arch";
    let detail = format!("{os}/{arch}");
    if !matches!(arch, "aarch64" | "arm64") {
        return Check::ok(ID, NAME, detail);
    }
    let fix = if os == "macos" {
        "nothing to do: Rosetta runs the amd64 images, they are only slower"
    } else {
        "an arm64 Linux host needs qemu/binfmt to run them: \
         `docker run --privileged --rm tonistiigi/binfmt --install amd64`"
    };
    Check::warn(
        ID,
        NAME,
        format!("{detail}; CHAP images are linux/amd64 only, so they run under emulation"),
        fix,
    )
}

/// Whether there is room for the images.
pub fn disk_verdict(free: Option<u64>) -> (Status, String, Option<String>) {
    let Some(free) = free else {
        return (
            Status::Skip,
            "free space could not be determined here".to_string(),
            None,
        );
    };
    let human = human_bytes(free);
    if free < DISK_FAIL {
        return (
            Status::Fail,
            format!("{human} free"),
            Some(format!(
                "free up space: the CHAP images total several GB, and below {} a pull stops \
                 halfway",
                human_bytes(DISK_FAIL)
            )),
        );
    }
    if free < DISK_WARN {
        return (
            Status::Warn,
            format!("{human} free"),
            Some(format!(
                "keep at least {} free: the CHAP images total several GB",
                human_bytes(DISK_WARN)
            )),
        );
    }
    (Status::Ok, format!("{human} free"), None)
}

/// The `disk` line, which also says which filesystem was measured.
pub fn disk_check(path: &Path, free: Option<u64>) -> Check {
    let (status, detail, fix) = disk_verdict(free);
    let detail = match status {
        Status::Skip => detail,
        _ => format!("{detail} on {}", path.display()),
    };
    Check::new("disk", "disk space", status, detail, fix)
}

/// `n` as a human-sized string. Always GB: every threshold here is one.
pub fn human_bytes(n: u64) -> String {
    format!("{:.1} GB", n as f64 / GB as f64)
}

/// Bytes available, from the output of `df -Pk PATH`.
///
/// `-P` is what makes this parseable: POSIX output puts every filesystem on
/// one line, where the plain format wraps a long device name onto its own.
/// The fourth column is `Available`, in 1024-byte blocks.
pub fn parse_df(text: &str) -> Option<u64> {
    let line = text.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    let blocks: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    blocks.checked_mul(1024)
}

/// Bytes available, from a PowerShell `(Get-PSDrive -Name C).Free`.
pub fn parse_free_bytes(text: &str) -> Option<u64> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .parse()
        .ok()
}

/// Free space on the filesystem holding `path`, or `None` when this platform
/// would not say.
fn free_bytes(path: &Path) -> Option<u64> {
    let (program, args, parse) = disk_command(path)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match run_bounded(program, &args, DOCKER_TIMEOUT) {
        Outcome::Done {
            ok: true, stdout, ..
        } => parse(&stdout),
        _ => None,
    }
}

/// How this platform is asked for free space, and how its answer is read.
///
/// `std::fs` has no free-space call and one number is not worth a crate, so
/// each platform is asked with the tool it already has: `df -Pk` on Unix,
/// PowerShell's `Get-PSDrive` on Windows. `cfg!` rather than `#[cfg]`, so both
/// parsers are compiled - and tested - on every platform.
type DiskCommand = (&'static str, Vec<String>, fn(&str) -> Option<u64>);

fn disk_command(path: &Path) -> Option<DiskCommand> {
    if cfg!(windows) {
        // `Get-PSDrive` is named after a drive letter, which is the first
        // component of any absolute Windows path.
        let drive = path
            .to_string_lossy()
            .chars()
            .next()
            .filter(char::is_ascii_alphabetic)?;
        return Some((
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                format!("(Get-PSDrive -Name {drive}).Free"),
            ],
            parse_free_bytes,
        ));
    }
    Some((
        "df",
        vec!["-Pk".to_string(), path.to_string_lossy().into_owned()],
        parse_df,
    ))
}

/// The engine version and the data root, split out of what [`INFO_FORMAT`]
/// asked for. Either may be missing: an engine that does not know a field
/// prints it empty rather than dropping it.
pub fn info_fields(stdout: &str) -> (String, Option<String>) {
    let mut fields = stdout.trim().split('\t');
    let version = fields.next().unwrap_or_default().trim().to_string();
    let root = fields
        .next()
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(str::to_string);
    (version, root)
}

/// The filesystem to measure: Docker's own data root when this host can see
/// it, and the working directory otherwise.
///
/// Docker Desktop reports a path inside its Linux VM (`/var/lib/docker`),
/// which says nothing about the disk out here; a root this host cannot see is
/// therefore not the one to measure.
pub fn disk_path(root: Option<&str>) -> PathBuf {
    if let Some(root) = root {
        let dir = PathBuf::from(root);
        if dir.is_dir() {
            return dir;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// What this build of `chaps` is, and whether there is a newer one.
///
/// Under `--offline` the lookup never happened, so the line is a skip that
/// says so and still names the build: "there is no update" is not something
/// this run is in a position to claim.
pub fn chaps_check(latest: ReleaseList<'_>) -> Check {
    const ID: &str = "chaps";
    let path = std::env::current_exe().ok();
    let method = path
        .as_deref()
        .map(selfupdate::install_method)
        .unwrap_or("release archive");
    let detail = format!("v{VERSION}, {TARGET}, {method}");
    match latest {
        ReleaseList::Newest(tag) if selfupdate::is_newer_than_current(tag) => Check::warn(
            ID,
            ID,
            format!("{detail}; {tag} is available"),
            "run `chaps self update`",
        ),
        ReleaseList::Offline => {
            Check::skip(ID, ID, format!("{detail}; {}", ReleaseList::OFFLINE_WHY))
        }
        _ => Check::ok(ID, ID, detail),
    }
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

/// The four lookups that need the network, still running.
struct Probes<'s> {
    ghcr: std::thread::ScopedJoinHandle<'s, std::result::Result<u16, String>>,
    marketplace: std::thread::ScopedJoinHandle<'s, std::result::Result<u16, String>>,
    chap_core: std::thread::ScopedJoinHandle<'s, std::result::Result<String, String>>,
    chaps: std::thread::ScopedJoinHandle<'s, std::result::Result<String, String>>,
}

/// The same four, answered.
///
/// Two of them do double duty: the release lookups are what tells the
/// `net-releases` and `chaps` lines that the host answered, and the tags they
/// came back with are what the pin checks compare against.
pub struct Probed {
    pub ghcr: std::result::Result<u16, String>,
    pub marketplace: std::result::Result<u16, String>,
    pub chap_core: std::result::Result<String, String>,
    pub chaps: std::result::Result<String, String>,
}

impl<'s> Probes<'s> {
    /// Start all four at once. Four sequential three-second timeouts would be
    /// twelve seconds of a command that has to stay interactive.
    fn spawn(scope: &'s std::thread::Scope<'s, '_>, registry_url: &str) -> Probes<'s> {
        let registry_url = registry_url.to_string();
        Probes {
            ghcr: scope.spawn(|| probe(GHCR_PROBE_URL, NET_TIMEOUT)),
            marketplace: scope.spawn(move || probe(&registry_url, NET_TIMEOUT)),
            chap_core: scope
                .spawn(|| chapcore::latest_release(NET_TIMEOUT).map_err(|e| format!("{e:#}"))),
            chaps: scope.spawn(|| {
                selfupdate::latest_release(NET_TIMEOUT)
                    .map(|release| release.tag)
                    .map_err(|e| format!("{e:#}"))
            }),
        }
    }

    /// Wait for all four. A panicked probe reads as an unreachable host,
    /// which is the verdict it would have produced anyway.
    fn join(self) -> Probed {
        fn joined<T>(
            handle: std::thread::ScopedJoinHandle<'_, std::result::Result<T, String>>,
        ) -> std::result::Result<T, String> {
            handle
                .join()
                .unwrap_or_else(|_| Err("the probe did not finish".to_string()))
        }
        Probed {
            ghcr: joined(self.ghcr),
            marketplace: joined(self.marketplace),
            chap_core: joined(self.chap_core),
            chaps: joined(self.chaps),
        }
    }
}

/// What one release lookup came back with, or why it came back with nothing.
///
/// The two empty cases are kept apart because they read differently on the
/// checklist: `--offline` is a flag the user set, and a check it held back
/// says so whatever else is installed on the machine; a lookup that was made
/// and did not arrive is a fact about the network instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseList<'a> {
    /// The newest tag the host named.
    Newest(&'a str),
    /// `--offline`: the list was never asked for.
    Offline,
    /// It was asked for and did not arrive.
    Unreachable,
}

impl<'a> ReleaseList<'a> {
    /// The reason every `--offline` line gives, worded the same way in all of
    /// them so a reader can tell the flag from a broken network at a glance.
    pub const OFFLINE_WHY: &'static str = "--offline: the release list was not asked";

    /// One joined probe, where `None` is `--offline`: that is the only reason
    /// a probe was never started.
    pub fn of(probe: Option<&'a std::result::Result<String, String>>) -> ReleaseList<'a> {
        match probe {
            None => ReleaseList::Offline,
            Some(Ok(tag)) => ReleaseList::Newest(tag),
            Some(Err(_)) => ReleaseList::Unreachable,
        }
    }
}

/// Ask one host whether it is there, within `timeout`.
///
/// A GET whose body is never read, which costs the same as a HEAD and works
/// on the endpoints that answer HEAD with 405. Any HTTP status counts as an
/// answer: ghcr.io replies 401 to an anonymous `/v2/`, and a host that
/// refuses us is still a host this machine can reach.
pub fn probe(url: &str, timeout: Duration) -> std::result::Result<u16, String> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(USER_AGENT)
            .build(),
    );
    let started = Instant::now();
    let answer = match agent.get(url).call() {
        Ok(response) => Ok(response.status().as_u16()),
        Err(ureq::Error::StatusCode(code)) => Ok(code),
        Err(err) => Err(err.to_string()),
    };
    crate::output::verbose(&format!(
        "GET {url} -> {} in {}ms",
        match &answer {
            Ok(code) => code.to_string(),
            Err(err) => err.clone(),
        },
        started.elapsed().as_millis()
    ));
    answer
}

/// One line per host CHAP needs, or three skipped lines under `--offline`.
pub fn network_checks(probed: Option<&Probed>) -> Vec<Check> {
    let hosts: [(&str, &str, &str, &str); 3] = [
        (
            "net-ghcr",
            "network ghcr.io",
            "the image registry",
            "without ghcr.io no image can be pulled; `--offline` keeps chaps itself working \
             from the embedded catalogue",
        ),
        (
            "net-marketplace",
            "network marketplace",
            "the model catalogue",
            "the catalogue falls back to the cache and then to the snapshot built into this \
             binary; `chaps registry show` says which one is in use",
        ),
        (
            "net-releases",
            "network releases",
            "the chap-core release list",
            "`chaps update` needs it; everything else works without it",
        ),
    ];
    let Some(probed) = probed else {
        return hosts
            .iter()
            .map(|(id, name, what, _)| {
                Check::skip(*id, *name, format!("--offline: {what} was not probed"))
            })
            .collect();
    };
    let answers: [&dyn ReachSummary; 3] = [&probed.ghcr, &probed.marketplace, &probed.chap_core];
    hosts
        .iter()
        .zip(answers)
        .map(|((id, name, what, fix), answer)| match answer.reached() {
            Ok(detail) => Check::ok(*id, *name, detail),
            Err(why) => Check::warn(*id, *name, format!("{what} is unreachable: {why}"), *fix),
        })
        .collect()
}

/// How a probe result reads on the line for its host.
///
/// Two of the probes fetch a document rather than only touching the host, so
/// each one describes its own success in its own terms.
pub trait ReachSummary {
    fn reached(&self) -> std::result::Result<String, String>;
}

impl ReachSummary for std::result::Result<u16, String> {
    fn reached(&self) -> std::result::Result<String, String> {
        match self {
            Ok(code) => Ok(format!("reachable (HTTP {code})")),
            Err(why) => Err(why.clone()),
        }
    }
}

impl ReachSummary for std::result::Result<String, String> {
    fn reached(&self) -> std::result::Result<String, String> {
        match self {
            Ok(tag) => Ok(format!("reachable, newest chap-core is {tag}")),
            Err(why) => Err(why.clone()),
        }
    }
}

// ---------------------------------------------------------------------------
// Project checks
// ---------------------------------------------------------------------------

/// The files a deployment directory holds, as `chaps init` writes them.
///
/// The base stack is only one of them when chap-core is a component of this
/// deployment: `--without chap-core` renders neither `compose.yml` nor the
/// chaps-owned override, so looking for them would report a deployment that is
/// exactly as asked for as broken.
pub fn project_files(components: &Components) -> Vec<String> {
    let mut files = vec![
        format!("{CHAPS_DIR}/{PROJECT_FILE}"),
        format!("{CHAPS_DIR}/{MODELS_FILE}"),
        format!("{CHAPS_DIR}/{COMPONENTS_FILE}"),
        ENV_FILE.to_string(),
    ];
    if components.chap_core.enabled {
        files.push(BASE_COMPOSE.to_string());
        files.push(CHAPS_COMPOSE.to_string());
    }
    files.extend(components.compose_files());
    files.push(MARKETPLACE_COMPOSE.to_string());
    files
}

/// Whether every file a deployment is made of is still there.
pub fn files_verdict(total: usize, missing: &[String]) -> (Status, String, Option<String>) {
    if missing.is_empty() {
        return (Status::Ok, format!("all {total} present"), None);
    }
    (
        Status::Fail,
        format!("missing {}", missing.join(", ")),
        Some(
            "run `chaps sync` to render the compose files again, or `chaps init --force` here \
             to write the whole deployment"
                .to_string(),
        ),
    )
}

/// The `project-files` line for a directory on disk.
pub fn files_check(dir: &Path, components: &Components) -> Check {
    let wanted = project_files(components);
    let missing: Vec<String> = wanted
        .iter()
        .filter(|name| !dir.join(name).is_file())
        .cloned()
        .collect();
    Check::from_verdict(
        "project-files",
        "project files",
        files_verdict(wanted.len(), &missing),
    )
}

/// What the `components` line says about the enabled set and the files that go
/// with it.
///
/// Everything about the `ocs` component that is on disk rather than in
/// `components.yaml`, gathered once so the verdict is a pure function of it.
#[derive(Debug, Clone, Default)]
pub struct OcsFacts<'a> {
    /// The body of `ocs/climate-service.yaml`, or `None` when there is none.
    pub config: Option<&'a str>,
    /// Whether `.env` sets any of the dataset credential variables.
    pub credentials: bool,
    /// How many files `ocs/plugins/` holds, or `None` when the project has no
    /// plugin directory at all.
    pub plugins: Option<usize>,
    /// How many datasets the running instance holds, from its JSON API, or
    /// `None` when it was not running or did not answer.
    pub datasets: Option<u32>,
    /// How much its data directory holds, in bytes, read from inside the
    /// running container. `None` while it is not running: what the volume
    /// holds is the `volumes` line's to report, where the question is what a
    /// `down --volumes` would destroy.
    pub data_bytes: Option<u64>,
}

/// `facts.config` is what is on disk at `ocs/climate-service.yaml`: `None` when
/// there is none, which is a real fault (the container would start with no
/// instance configuration), and a body that still carries the example marker,
/// which is a warning - it deploys, it just deploys Sierra Leone.
///
/// The credentials and the plugin count are only ever reported, never judged. A
/// deployment with no dataset credentials is a working deployment: WorldPop and
/// CHIRPS3 need none, and an operator who only wants those should not be told
/// once a day that something is unset on purpose.
pub fn components_verdict(
    components: &Components,
    facts: &OcsFacts,
) -> (Status, String, Option<String>) {
    let label = components.label();
    if !components.ocs.enabled {
        return (Status::Ok, label, None);
    }
    let config = format!("{OCS_DIR}/{OCS_CONFIG_FILE}");
    let Some(body) = facts.config else {
        return (
            Status::Fail,
            format!("{label}; {config} is missing"),
            Some(
                "run `chaps sync` to scaffold it again, then edit it for your country".to_string(),
            ),
        );
    };
    if body.contains(OCS_EXAMPLE_MARKER) {
        return (
            Status::Warn,
            format!("{label}; {config} still holds OCS's example values"),
            Some(format!(
                "edit {config} for your own country or region and delete the note at the top; \
                 `chaps components enable ocs --ocs-name NAME --ocs-country CODE --ocs-bbox \
                 xmin,ymin,xmax,ymax` writes it for a project that has none"
            )),
        );
    }
    (
        Status::Ok,
        format!("{label}; {config} present{}", ocs_notes(facts)),
        None,
    )
}

/// The informational tail of the `components` line: what the OCS instance has
/// beyond its config file.
fn ocs_notes(facts: &OcsFacts) -> String {
    let mut notes = String::new();
    if !facts.credentials {
        notes.push_str("; ERA5-Land: credentials unset (WorldPop and CHIRPS3 work without them)");
    }
    if let Some(count) = facts.plugins {
        notes.push_str(&format!(
            "; {OCS_PLUGINS_DIR}/: {count} file{}",
            if count == 1 { "" } else { "s" }
        ));
    }
    if let Some(count) = facts.datasets {
        notes.push_str(&format!(
            "; {count} dataset{}",
            if count == 1 { "" } else { "s" }
        ));
    }
    if let Some(bytes) = facts.data_bytes {
        notes.push_str(&format!("; {} data", crate::backup::human_size(bytes)));
    }
    notes
}

/// The `components` line for a project on disk.
///
/// `running` is the set of compose services that are up, which is what decides
/// whether the instance is asked what it holds: a dataset count and a data
/// size are read out of a container that is there, and neither is worth a
/// timeout spent on one that is not.
pub fn components_check(project: &Project, running: &BTreeSet<String>) -> Check {
    let body = std::fs::read_to_string(project.ocs_config_path()).ok();
    let env = std::fs::read_to_string(project.dir.join(ENV_FILE)).unwrap_or_default();
    let live =
        project.state.components.ocs.enabled && running.contains(crate::compose::OCS_SERVICE);
    let facts = OcsFacts {
        config: body.as_deref(),
        credentials: crate::components::OCS_DATA_SOURCE_ENV_VARS
            .iter()
            .any(|var| crate::dotenv::non_empty(&env, var).is_some()),
        plugins: plugin_count(&project.ocs_plugins_path()),
        datasets: live
            .then(|| project.state.components.ocs_url())
            .flatten()
            .and_then(|url| crate::status::ocs_datasets(&url)),
        data_bytes: live.then(|| docker::ocs_data_bytes(project)).flatten(),
    };
    Check::from_verdict(
        "components",
        "components",
        components_verdict(&project.state.components, &facts),
    )
}

/// How many files the plugin directory holds, or `None` when there is none.
///
/// Files at any depth, because OCS's own layout is `plugins/datasets/*.py`: a
/// count of the top level would read `1` for a directory with a dozen datasets
/// in it.
fn plugin_count(dir: &Path) -> Option<usize> {
    if !dir.is_dir() {
        return None;
    }
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(entry.path()),
                Ok(_) => count += 1,
                Err(_) => {}
            }
        }
    }
    Some(count)
}

/// One line per model this deployment added itself that follows a branch:
/// is the build it runs still the newest one that branch has published.
///
/// Nothing here is a failure. A model added from an image reference is pinned
/// on purpose and only reported as such, and a repository that will not
/// answer is a check that was not made rather than a problem with the
/// deployment.
fn manual_checks(ctx: &Ctx, project: &Project) -> Vec<Check> {
    let endpoints = crate::manual::Endpoints {
        timeout: NET_TIMEOUT,
        ..crate::manual::Endpoints::from_env(ctx.registry.offline)
    };
    project
        .state
        .manual
        .iter()
        .map(|(id, entry)| {
            // The lookup is only made for an entry that has a branch to
            // compare against, and only when this run may use the network.
            let newest = match (&entry.repository, &entry.follow, endpoints.offline) {
                (Some(repository), Some(branch), false) => {
                    crate::manual::newest_published(repository, branch, &endpoints)
                        .ok()
                        .flatten()
                        .map(|(_, tag)| tag)
                }
                _ => None,
            };
            manual_check(id, entry, newest.as_deref(), endpoints.offline)
        })
        .collect()
}

/// The verdict for one manually added model, given what its branch publishes
/// now.
fn manual_check(
    id: &str,
    entry: &crate::project::ManualModel,
    newest: Option<&str>,
    offline: bool,
) -> Check {
    let check_id = format!("manual-{id}");
    let name = format!("manual {id}");
    let Some(branch) = &entry.follow else {
        return Check::skip(
            check_id,
            name,
            format!("pinned to {}, so nothing moves it", entry.tag),
        );
    };
    if offline {
        return Check::skip(
            check_id,
            name,
            format!("--offline, so {branch} was not asked about a newer build"),
        );
    }
    match newest {
        None => Check::skip_with(
            check_id,
            name,
            format!(
                "could not ask {} for the newest build on {branch}",
                entry
                    .repository
                    .clone()
                    .unwrap_or_else(|| entry.image.clone())
            ),
            "run `chaps update --dry-run` to see the error in full",
        ),
        Some(tag) if tag == entry.tag => Check::ok(
            check_id,
            name,
            format!("{tag}, the newest published build on {branch}"),
        ),
        Some(tag) => Check::warn(
            check_id,
            name,
            format!("running {}, and {branch} has published {tag}", entry.tag),
            "run `chaps update` to move the pin",
        ),
    }
}

/// Whether the rendered compose files still match `.chaps/`.
///
/// This is `chaps sync --check` with its own exit code taken away: the same
/// dry run, reported as one line.
fn sync_check(ctx: &Ctx, project: &Project) -> Check {
    const ID: &str = "sync";
    const NAME: &str = "compose files";
    // The same loader `chaps sync` uses, so this reports the same verdict,
    // but with the probe timeout rather than the registry's: a cold cache
    // must not be able to hold the checklist open.
    let options = registry::RegistryOptions {
        timeout: NET_TIMEOUT,
        ..ctx.registry.clone()
    };
    let registry = match registry::load(&options) {
        Ok(mut registry) => {
            // The deployment's own definitions too, or a manually added model
            // would look like one the catalogue dropped. A collision is
            // reported by every command that writes; this one only reads.
            let _ = registry.with_manual(&project.state.manual);
            registry
        }
        Err(err) => {
            return Check::skip(ID, NAME, format!("no catalogue to compare against: {err}"));
        }
    };
    // A clone because `sync` takes the project by value to update its
    // rendered-file list; in check mode it writes nothing, and this copy is
    // thrown away either way.
    let mut project = project.clone();
    match sync(&mut project, &registry, true) {
        Err(err) => Check::fail(
            ID,
            NAME,
            format!("{err:#}"),
            "run `chaps sync` to see it in full",
        ),
        Ok(report) if report.drift => Check::warn(
            ID,
            NAME,
            format!("out of date with .chaps/ ({})", report.summary()),
            "run `chaps sync`, or `chaps up`, which syncs first",
        ),
        Ok(report) => Check::ok(ID, NAME, format!("in sync ({})", report.summary())),
    }
}

/// The compose project name, and whether it is this deployment's own.
///
/// Compose names a project after its directory unless a file says otherwise,
/// so two deployments in directories both called `demo` share every container
/// name and every named volume - a fresh `chaps up` in the second one finds
/// the first one's database, with a password it has never seen. A deployment
/// written by this version of `chaps` records a name of its own; an older one
/// has the directory name and nothing else, which is what this warns about.
pub fn project_name_verdict(
    recorded: Option<&str>,
    dir_name: &str,
) -> (Status, String, Option<String>) {
    if let Some(name) = recorded {
        return (
            Status::Ok,
            format!("{name} (recorded in {CHAPS_DIR}/{PROJECT_FILE})"),
            None,
        );
    }
    let derived = crate::project::normalized_project_name(dir_name).unwrap_or_default();
    (
        Status::Warn,
        format!(
            "compose project name is the directory name; volumes can collide with other \
             deployments named {dir_name}"
        ),
        Some(format!(
            "run `chaps sync` to record it as `{derived}` in {CHAPS_DIR}/{PROJECT_FILE}; \
             it is the name compose already uses, so nothing is renamed"
        )),
    )
}

/// The `project` line.
pub fn project_check(project: &Project) -> Check {
    let dir_name = project
        .dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Check::from_verdict(
        "compose-project",
        "project",
        project_name_verdict(project.compose_project(), &dir_name),
    )
}

/// Whether the named volumes this deployment would use are its own.
///
/// `created` is when `.chaps/project.yaml` was created and each volume's is
/// when docker created it. A database volume older than the deployment
/// directory itself was made by something else - an earlier deployment of the
/// same name, most often - and it still holds that deployment's role password,
/// which is exactly the state in which chap-core cannot log in.
///
/// Both times are optional: a filesystem that does not record a creation time,
/// and a docker that did not say, each cost the comparison and nothing else.
///
/// `ocs_data` is the OCS data volume and what it holds, when that could be
/// measured: the one volume under this prefix whose size is usually worth
/// knowing, because it is what a `chaps down --volumes` would destroy.
pub fn volume_verdict(
    prefix: &str,
    db_volume: &str,
    volumes: &[(String, Option<u64>)],
    created: Option<u64>,
    leftover: &[String],
    ocs_data: Option<(&str, u64)>,
) -> (Status, String, Option<String>) {
    if volumes.is_empty() {
        return (
            Status::Ok,
            format!("no {prefix}* volume yet; `chaps up` creates them"),
            None,
        );
    }
    let held = match ocs_data {
        Some((name, bytes)) => format!("; {name} holds {}", crate::backup::human_size(bytes)),
        None => String::new(),
    };
    let count = format!(
        "{} volume{} named {prefix}*{held}",
        volumes.len(),
        if volumes.len() == 1 { "" } else { "s" }
    );
    let older = volumes.iter().find(|(name, at)| {
        name == db_volume && matches!((at, created), (Some(a), Some(c)) if *a < c)
    });
    // The database one first: a deployment that cannot log in is a stack that
    // does not come up, where a volume nobody reads any more costs disk.
    if let Some((name, at)) = older {
        return (
            Status::Warn,
            format!(
                "the database volume {name} predates this deployment; if chap-core cannot log \
                 in, it belongs to an earlier deployment with the same name (volume {}, \
                 deployment {})",
                crate::backup::timestamp(at.unwrap_or_default()),
                crate::backup::timestamp(created.unwrap_or_default())
            ),
            Some(
                "remove it with `chaps down --volumes` if this deployment's data can go, \
                 or keep both by giving one of them a name of its own"
                    .to_string(),
            ),
        );
    }
    if !leftover.is_empty() {
        return (
            Status::Warn,
            format!(
                "leftover volumes from disabled models or components: {}{held}",
                leftover.join(", ")
            ),
            Some(LEFTOVER_FIX.to_string()),
        );
    }
    (Status::Ok, count, None)
}

/// What to do about the leftover volumes the check found.
///
/// `down --volumes` is not among the answers on purpose: it only removes the
/// volumes the compose files still declare, which is exactly the set these are
/// not in.
const LEFTOVER_FIX: &str = "remove each with `chaps models disable <id> --purge` or `chaps components disable <name> \
     --purge`, or `docker volume rm <name>`; keep them to have the data back when the model or \
     component is enabled again";

/// The volumes under this deployment's prefix that belong to nothing it still
/// enables.
///
/// A model's volume is `ck_<id>_data`, so a name of that shape whose id is no
/// longer in `.chaps/models.yaml` is a disabled model's data; the component
/// ones are named outright, and are leftovers while the component is off.
/// Every other name - the database, chap-core's own - belongs to the base
/// stack, which is nobody's leftover.
///
/// Pure, and injected with both lists: the verdict is decided here and the
/// docker call that gathers the names is [`volumes_check`]'s.
pub fn leftover_volumes(
    prefix: &str,
    volumes: &[String],
    models: &[String],
    components: &Components,
) -> Vec<String> {
    volumes
        .iter()
        .filter(|name| {
            let Some(bare) = name.strip_prefix(prefix) else {
                return false;
            };
            if let Some(id) = crate::compose::volume_model_id(bare) {
                return !models.iter().any(|enabled| enabled == id);
            }
            Component::ALL.iter().any(|component| {
                component.volume() == Some(bare) && !components.is_enabled(*component)
            })
        })
        .cloned()
        .collect()
}

/// The `volumes` line: what docker holds under this deployment's name.
///
/// `running` decides whether the OCS data volume is measured here: while its
/// container is up, the `components` line has the size from inside it, and
/// starting a second container to measure what the first is writing to would
/// be both slower and less true. While it is down, this is the line that says
/// how much data a `chaps down --volumes` would destroy.
fn volumes_check(project: &Project, running: &BTreeSet<String>) -> Check {
    const ID: &str = "volumes";
    const NAME: &str = "volumes";
    let Some(prefix) = project.volume_prefix() else {
        return Check::skip(ID, NAME, "this directory has no compose project name");
    };
    let volumes: Vec<(String, Option<u64>)> = docker::volumes_with_prefix(&prefix)
        .into_iter()
        .map(|v| (v.name, crate::status::parse_rfc3339(&v.created_at)))
        .collect();
    let created = deployment_created_at(project);
    let names: Vec<String> = volumes.iter().map(|(name, _)| name.clone()).collect();
    let models: Vec<String> = project.state.models.keys().cloned().collect();
    let leftover = leftover_volumes(&prefix, &names, &models, &project.state.components);
    let ocs_volume = format!("{prefix}{}", crate::compose::render::OCS_VOLUME);
    let ocs_data = (!running.contains(crate::compose::OCS_SERVICE) && names.contains(&ocs_volume))
        .then(|| docker::volume_size_bytes(&ocs_volume))
        .flatten()
        .map(|bytes| (ocs_volume.as_str(), bytes));
    Check::from_verdict(
        ID,
        NAME,
        volume_verdict(
            &prefix,
            &format!("{prefix}chap-db"),
            &volumes,
            created,
            &leftover,
            ocs_data,
        ),
    )
}

/// When this deployment directory was created, in Unix seconds.
///
/// The `.chaps/` directory rather than `project.yaml` inside it: every save
/// writes that file to a temporary sibling and renames it into place, so its
/// creation time is the time of the last `chaps sync` and not the time the
/// deployment was made. The directory is created once by `chaps init` and
/// never replaced.
///
/// `None` where the filesystem records no creation time, and the check that
/// uses it simply does not make the comparison.
fn deployment_created_at(project: &Project) -> Option<u64> {
    let chaps = project.chaps_dir();
    file_created_at(&chaps).or_else(|| file_created_at(&chaps.join(PROJECT_FILE)))
}

/// When a file or directory was created, in Unix seconds, when the filesystem
/// records it.
fn file_created_at(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|m| m.created())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// What `.env` says about the two things that bite later.
///
/// The default database password is the one a deployment reachable from
/// anywhere but the loopback should not keep, and a missing `CHAP_IMAGE_TAG`
/// line means the pin comments `sync` writes are gone. Whether authentication
/// is on is reported either way: "off" is a fact an operator has to be able to
/// see, not a fault.
///
/// `chaps_overlay` is the rendered `compose.chaps.yml`, which is where the
/// registration key is handed to chap-core: a protected deployment whose
/// override predates that line has a chap-core that answers every model's
/// registration with a 401.
pub fn env_verdict(
    body: Option<&str>,
    chaps_overlay: Option<&str>,
) -> (Status, String, Option<String>) {
    let Some(body) = body else {
        return (Status::Skip, "no .env to read".to_string(), None);
    };
    let auth_on = auth::active_value(body, auth::API_TOKEN_ENV_VAR).is_some();
    let password = auth::active_value(body, "POSTGRES_PASSWORD");
    let tag_line = body.lines().any(|line| {
        line.trim_start()
            .trim_start_matches('#')
            .trim_start()
            .starts_with(&format!("{CHAP_TAG_ENV_VAR}="))
    });

    let mut facts = vec![format!("auth {}", if auth_on { "on" } else { "off" })];
    let mut fixes = Vec::new();
    match password.as_deref() {
        None => {
            facts.push("POSTGRES_PASSWORD unset, so compose uses the default `chap`".to_string());
            fixes.push(PASSWORD_FIX);
        }
        Some("chap") => {
            facts.push("POSTGRES_PASSWORD is the default `chap`".to_string());
            fixes.push(PASSWORD_FIX);
        }
        Some(_) => facts.push("POSTGRES_PASSWORD set".to_string()),
    }
    if tag_line {
        facts.push(format!("{CHAP_TAG_ENV_VAR} present"));
    } else {
        facts.push(format!("no {CHAP_TAG_ENV_VAR} line"));
        fixes.push("run `chaps sync` to write the image pin comments back");
    }
    // Only a deployment with authentication on has anything to hand over, and
    // only an override rendered by an older chaps is missing the line.
    if auth_on
        && let Some(overlay) = chaps_overlay
        && !overlay.contains(auth::REGISTRATION_KEY_ENV_VAR)
    {
        facts.push(format!(
            "{CHAPS_COMPOSE} does not pass {} to chap-core",
            auth::REGISTRATION_KEY_ENV_VAR
        ));
        fixes.push(
            "run `chaps sync`, then `chaps restart`: without that line chap-core answers \
             every model registration with HTTP 401",
        );
    }

    let detail = facts.join(", ");
    if fixes.is_empty() {
        return (Status::Ok, detail, None);
    }
    (Status::Warn, detail, Some(fixes.join("; ")))
}

/// What to do about a database password anyone can guess.
const PASSWORD_FIX: &str = "set POSTGRES_PASSWORD in .env to a value of your own; a database volume that already \
     exists keeps the old password until `ALTER USER` changes it";

/// The `.env` line.
pub fn env_check(body: Option<&str>, chaps_overlay: Option<&str>) -> Check {
    Check::from_verdict("env", ".env", env_verdict(body, chaps_overlay))
}

/// Whether one host port the stack publishes is free to publish on.
///
/// `busy` is injected so the verdict can be tested without binding anything,
/// and the fix is the very sentence `chaps up`'s preflight would have failed
/// with, so the two never drift apart.
///
/// `note` is appended in parentheses to whatever the line says about the port:
/// [`api_port_note`] uses it to name the file that moved the API port, without
/// which the number looks wrong against `.chaps/project.yaml`.
pub fn port_check(
    claim: &PortClaim,
    running: &BTreeSet<String>,
    busy: &dyn Fn(u16) -> bool,
    suggestion: Option<u16>,
    note: Option<&str>,
) -> Check {
    let is_api = claim.service == API_SERVICE;
    let id = if is_api {
        "api-port".to_string()
    } else {
        format!("port-{}", claim.service)
    };
    let name = if is_api {
        "api port".to_string()
    } else {
        format!("port {}", claim.service)
    };
    let detail = |what: &str| match note {
        Some(note) => format!("{} {what} ({note})", claim.port),
        None => format!("{} {what}", claim.port),
    };
    if running.contains(&claim.service) {
        return Check::ok(id, name, detail("is held by this project's own container"));
    }
    if busy(claim.port) {
        return Check::fail(
            id,
            name,
            detail("is in use by something else"),
            ports::busy_line(claim, suggestion),
        );
    }
    Check::ok(id, name, detail("is free"))
}

/// What the `api port` line says about where its number came from, when that
/// is not what `.chaps/project.yaml` records.
///
/// compose reads `.env` last, so a `CHAP_API_PORT=` line there moves the
/// published port. The checklist has to name the file that did it: `18000 is
/// free` next to a recorded `api_port: 8000` otherwise reads as a bug in
/// `chaps` rather than as a deliberate override. The ordinary case, where the
/// two agree, says nothing.
pub fn api_port_note(project: &Project) -> Option<String> {
    let (port, source) = project.api_port_in_effect();
    (source == ApiPortSource::Env && port != project.state.api_port).then(|| {
        format!(
            "{}, over the {} recorded in .chaps/project.yaml",
            source.label(),
            project.state.api_port
        )
    })
}

/// Whether the chap-core tag this deployment pins is still the newest one.
///
/// A moving tag is answered before the release list is looked at: there is no
/// comparison to make, so that verdict is the same online and offline.
pub fn pin_verdict(tag: &str, latest: ReleaseList<'_>) -> (Status, String, Option<String>) {
    if chapcore::is_moving_tag(tag) {
        return (
            Status::Ok,
            format!("{tag} (a moving tag: `chaps up --pull` takes whatever it points at today)"),
            None,
        );
    }
    let latest = match latest {
        ReleaseList::Newest(latest) => latest,
        ReleaseList::Offline => {
            return (
                Status::Skip,
                format!("{tag} pinned; {}", ReleaseList::OFFLINE_WHY),
                None,
            );
        }
        ReleaseList::Unreachable => {
            return (
                Status::Skip,
                format!("{tag} pinned; the release list was not reachable"),
                None,
            );
        }
    };
    if chapcore::is_newer(latest, tag) {
        return (
            Status::Warn,
            format!("{tag} pinned, {latest} released"),
            Some("run `chaps update --dry-run` to see what would move".to_string()),
        );
    }
    (Status::Ok, format!("{tag} is the newest release"), None)
}

/// The `chap-core-pin` line.
pub fn pin_check(tag: &str, latest: ReleaseList<'_>) -> Check {
    Check::from_verdict("chap-core-pin", "chap-core pin", pin_verdict(tag, latest))
}

/// Whether a manifest document offers a linux/amd64 image.
///
/// `None` when the document says nothing about platforms at all: a single
/// image manifest carries no list, and "the tag exists" is then the whole
/// answer this can give.
pub fn manifest_has_amd64(body: &str) -> Option<bool> {
    let doc: serde_json::Value = serde_json::from_str(body).ok()?;
    let manifests = doc.get("manifests")?.as_array()?;
    Some(manifests.iter().any(|entry| {
        let platform = entry.get("platform");
        let field = |key: &str| {
            platform
                .and_then(|p| p.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        };
        field("os") == "linux" && field("architecture") == "amd64"
    }))
}

/// One enabled model's image, from what `docker manifest inspect` said.
pub fn image_verdict(
    marketplace_id: &str,
    service_id: &str,
    reference: &str,
    outcome: &Outcome,
) -> Check {
    let id = format!("image-{service_id}");
    let name = format!("image {service_id}");
    match outcome {
        Outcome::Done {
            ok: true, stdout, ..
        } => match manifest_has_amd64(stdout) {
            Some(false) => Check::warn(
                id,
                name,
                format!("{reference} has no linux/amd64 image"),
                format!(
                    "the overlay pins platform: linux/amd64, so this tag cannot run; \
                     re-resolve it with `chaps models enable {marketplace_id}`"
                ),
            ),
            Some(true) => Check::ok(id, name, format!("{reference} (linux/amd64)")),
            None => Check::ok(id, name, format!("{reference} exists")),
        },
        Outcome::TimedOut => Check::skip(
            id,
            name,
            format!(
                "ghcr.io did not answer for {reference} within {}s",
                MANIFEST_TIMEOUT.as_secs()
            ),
        ),
        Outcome::Missing => Check::skip(id, name, "no docker CLI to ask"),
        Outcome::Failed(why) => Check::skip(id, name, why.clone()),
        Outcome::Done { stderr, .. } if stderr.to_ascii_lowercase().contains("experimental") => {
            Check::skip(
                id,
                name,
                "this docker CLI has `docker manifest` behind its experimental flag",
            )
        }
        Outcome::Done { stderr, .. } => Check::fail(
            id,
            name,
            format!("{reference} was not found: {}", first_line(stderr)),
            format!("run `chaps models enable {marketplace_id}` to resolve the pin again"),
        ),
    }
}

/// Why the image manifests were not looked up, or `None` when they were.
///
/// `--offline` is answered first and on its own: it is the reason the user
/// gave, it holds whether or not this machine has a docker CLI, and a
/// checklist that said "no docker CLI to ask" on a runner without Docker but
/// "--offline" on a laptop with it would report the same run two ways. A
/// missing CLI is the reason only for a run that did go looking, where it is
/// what stopped the lookup; `docker-cli` has failed on its own line by then.
pub fn image_skip_reason(probed: Option<&Probed>, have_cli: bool) -> Option<String> {
    let Some(probed) = probed else {
        return Some("--offline: the registry was not asked".to_string());
    };
    if !have_cli {
        return Some("no docker CLI to ask".to_string());
    }
    probed
        .ghcr
        .as_ref()
        .err()
        .map(|why| format!("ghcr.io is unreachable: {why}"))
}

/// One enabled component's image, from what `docker manifest inspect` said.
///
/// Unlike a model overlay, a component pins no platform: both images are
/// multi-arch, so "the tag exists" is the whole question here.
pub fn component_image_verdict(name: &str, reference: &str, outcome: &Outcome) -> Check {
    let id = format!("image-{name}");
    let check_name = format!("image {name}");
    match outcome {
        Outcome::Done { ok: true, .. } => Check::ok(id, check_name, format!("{reference} exists")),
        Outcome::TimedOut => Check::skip(
            id,
            check_name,
            format!(
                "the registry did not answer for {reference} within {}s",
                MANIFEST_TIMEOUT.as_secs()
            ),
        ),
        Outcome::Missing => Check::skip(id, check_name, "no docker CLI to ask"),
        Outcome::Failed(why) => Check::skip(id, check_name, why.clone()),
        Outcome::Done { stderr, .. } if stderr.to_ascii_lowercase().contains("experimental") => {
            Check::skip(
                id,
                check_name,
                "this docker CLI has `docker manifest` behind its experimental flag",
            )
        }
        Outcome::Done { stderr, .. } => Check::fail(
            id,
            check_name,
            format!("{reference} was not found: {}", first_line(stderr)),
            format!(
                "check the tag: `{OCS_TAG_ENV_VAR}` and `{S3_TAG_ENV_VAR}` in .env pin the \
                 component images"
            ),
        ),
    }
}

/// The images the enabled components pull, as `(component, reference)`.
pub fn component_images(components: &Components) -> Vec<(String, String)> {
    let mut images = Vec::new();
    if components.ocs.enabled {
        images.push((
            crate::compose::OCS_SERVICE.to_string(),
            format!("{OCS_IMAGE}:{}", components.ocs.image_tag),
        ));
    }
    if components.s3.enabled {
        images.push((
            crate::compose::S3_SERVICE.to_string(),
            format!("{S3_IMAGE}:{S3_DEFAULT_TAG}"),
        ));
    }
    images
}

/// One line per enabled model: does the user its overlay runs it as still
/// match what the image declares.
///
/// The one class of breakage `chaps sync` cannot see: the recorded user is
/// what the overlay renders from, and an image that runs as root under a
/// `user: 1000:1000` starts fine and then fails on its own binaries - the
/// Rwanda BYM model's `inla.run: Permission denied`. Only a locally pulled
/// image can be asked without a network round trip per model, so a model whose
/// amd64 image is not in this machine's image store is skipped rather than
/// guessed at.
fn user_checks(project: &Project, have_cli: bool) -> Vec<Check> {
    user_checks_with(project, have_cli, &crate::docker::image_config)
}

/// [`user_checks`] with the daemon injected, so the verdicts can be tested.
fn user_checks_with(
    project: &Project,
    have_cli: bool,
    declared: &dyn Fn(&str) -> Option<(String, String)>,
) -> Vec<Check> {
    project
        .state
        .models
        .iter()
        .map(|(id, model)| {
            let reference = crate::compose::image_ref(&model.image, &model.image_tag);
            let found = have_cli
                .then(|| declared(&reference))
                .flatten()
                .map(|(user, _)| user);
            let asked = have_cli.then_some(reference.as_str());
            user_check(id, &model.service_id, asked, &model.user, found.as_deref())
        })
        .collect()
}

/// The verdict for one model, given what its image declares.
///
/// `declared` is `None` when the image the overlay pins is not in this
/// machine's image store as the `linux/amd64` variant every overlay runs -
/// the usual state before the first `chaps up`, and no reason to say anything
/// is wrong. `reference` is the tag that was asked about, or `None` when
/// there was no docker CLI to ask through.
pub fn user_check(
    id: &str,
    service_id: &str,
    reference: Option<&str>,
    recorded: &str,
    declared: Option<&str>,
) -> Check {
    let check_id = format!("user-{service_id}");
    let name = format!("user {service_id}");
    let Some(declared) = declared else {
        let Some(reference) = reference else {
            return Check::skip(check_id, name, format!("{recorded}; no docker CLI to ask"));
        };
        return Check::skip_with(
            check_id,
            name,
            format!("{recorded}; the amd64 variant of {reference} is not in the local image store"),
            format!("run `chaps docker pull`, or `docker pull --platform linux/amd64 {reference}`"),
        );
    };
    let wanted = crate::compose::resolve::normalize(declared);
    // An account name only the image itself can resolve says nothing about
    // whether the numbers beside it are right.
    if crate::compose::overrides::numeric_pair(&wanted).is_none() {
        return Check::skip(
            check_id,
            name,
            format!("the image runs as `{wanted}`, which only the image can turn into numbers"),
        );
    }
    if crate::compose::resolve::normalize(recorded) == wanted {
        return Check::ok(check_id, name, format!("{recorded}, as the image declares"));
    }
    Check::warn(
        check_id,
        name,
        format!("{recorded}, but the image runs as {wanted}"),
        format!(
            "run `chaps models enable {id}` to read the user off the image again, or `chaps update`"
        ),
    )
}

/// One line per enabled model, asked in parallel.
fn image_checks(project: &Project, probed: Option<&Probed>, have_cli: bool) -> Vec<Check> {
    // Marketplace id, service id and the exact reference the overlay pins.
    let models: Vec<(String, String, String)> = project
        .state
        .models
        .iter()
        .map(|(id, model)| {
            (
                id.clone(),
                model.service_id.clone(),
                crate::compose::image_ref(&model.image, &model.image_tag),
            )
        })
        .collect();
    let components = component_images(&project.state.components);

    if let Some(reason) = image_skip_reason(probed, have_cli) {
        let model_skips = models.iter().map(|(_, service_id, _)| service_id);
        let component_skips = components.iter().map(|(name, _)| name);
        return model_skips
            .chain(component_skips)
            .map(|name| {
                Check::skip(
                    format!("image-{name}"),
                    format!("image {name}"),
                    reason.clone(),
                )
            })
            .collect();
    }

    // One thread per image: each is a registry round trip, and a deployment
    // with four models would otherwise wait out four of them in a row.
    std::thread::scope(|scope| {
        let references: Vec<String> = models
            .iter()
            .map(|(_, _, reference)| reference.clone())
            .chain(components.iter().map(|(_, reference)| reference.clone()))
            .collect();
        let handles: Vec<_> = references
            .iter()
            .map(|reference| {
                let reference = reference.clone();
                scope.spawn(move || {
                    run_bounded(
                        "docker",
                        &["manifest", "inspect", &reference],
                        MANIFEST_TIMEOUT,
                    )
                })
            })
            .collect();
        let mut outcomes = handles.into_iter().map(|handle| {
            handle
                .join()
                .unwrap_or_else(|_| Outcome::Failed("the probe did not finish".to_string()))
        });
        let mut checks: Vec<Check> = models
            .iter()
            .zip(outcomes.by_ref())
            .map(|((marketplace_id, service_id, reference), outcome)| {
                image_verdict(marketplace_id, service_id, reference, &outcome)
            })
            .collect();
        checks.extend(
            components
                .iter()
                .zip(outcomes)
                .map(|((name, reference), outcome)| {
                    component_image_verdict(name, reference, &outcome)
                }),
        );
        checks
    })
}

/// What a deployment without chap-core adds up to: its components alone.
fn components_only_verdict(report: &StatusReport) -> (Status, String, Option<String>) {
    let (up, down): (Vec<&str>, Vec<&str>) = report
        .components
        .iter()
        .map(|c| (c.name.as_str(), c.state))
        .fold((Vec::new(), Vec::new()), |(mut up, mut down), (name, s)| {
            if s == ComponentState::Up {
                up.push(name);
            } else {
                down.push(name);
            }
            (up, down)
        });
    if down.is_empty() {
        return (
            Status::Ok,
            format!("no chap-core here; up: {}", up.join(", ")),
            None,
        );
    }
    (
        Status::Warn,
        format!("no chap-core here; not up: {}", down.join(", ")),
        Some("run `chaps status` for what each component is doing".to_string()),
    )
}

/// What the running deployment adds up to.
pub fn stack_verdict(report: &StatusReport) -> (Status, String, Option<String>) {
    match &report.api {
        ApiHealth::Off => components_only_verdict(report),
        // A container that is up and unhealthy is a sharper answer than "the
        // port does not answer", and its own last error line is sharper still.
        ApiHealth::Down { .. } if report.api_container_unhealthy() => {
            let cause = crate::diagnose::headline(&report.unhealthy)
                .map(|line| format!(": {line}"))
                .unwrap_or_default();
            (
                Status::Fail,
                format!(
                    "the chap container is up and unhealthy{}",
                    first_line(&cause)
                ),
                Some(
                    report
                        .unhealthy
                        .iter()
                        .find_map(|entry| entry.hint.clone())
                        .unwrap_or_else(|| "run `chaps logs chap` to see why".to_string()),
                ),
            )
        }
        ApiHealth::Down { error } => (
            Status::Fail,
            // Whatever its container last said about itself beats the
            // connection error out here, which is only the symptom.
            match crate::diagnose::headline(&report.unhealthy) {
                Some(line) => format!(
                    "chap-core at {} is down: {}",
                    report.api_url,
                    first_line(&line)
                ),
                None => format!("chap-core at {} is down: {error}", report.api_url),
            },
            Some(
                report
                    .unhealthy
                    .iter()
                    .find_map(|entry| entry.hint.clone())
                    .unwrap_or_else(|| {
                        "run `chaps logs chap` to see why, and `chaps status` for the detail"
                            .to_string()
                    }),
            ),
        ),
        ApiHealth::Up { .. } if report.missing.is_empty() => (
            Status::Ok,
            format!(
                "chap-core up at {}, {} of {} models registered",
                report.api_url,
                report.expected.len(),
                report.expected.len()
            ),
            // Registration is a heartbeat, so a green line here is not proof
            // that any of these models can produce a prediction. There is a
            // check that settles it, and `doctor` cannot make it: it runs the
            // models, which takes minutes.
            (!report.expected.is_empty()).then(|| crate::status::TEST_HINT.to_string()),
        ),
        ApiHealth::Up { .. } => (
            Status::Warn,
            format!(
                "chap-core up at {}, not registered: {}",
                report.api_url,
                report.missing.join(", ")
            ),
            Some("run `chaps status` for what to do about each one".to_string()),
        ),
    }
}

/// The `stack` line, which has nothing to check when nothing is up.
fn stack_check(
    project: &Project,
    containers: Option<&[docker::Container]>,
    running: &BTreeSet<String>,
) -> Check {
    const ID: &str = "health";
    const NAME: &str = "health";
    if running.is_empty() {
        return Check::skip_with(
            ID,
            NAME,
            "no container of this project is running",
            "run `chaps up` to start CHAP",
        );
    }
    let url = project.api_url();
    crate::output::verbose(&format!("asking chap-core at {url}"));
    let token = auth::token_in(&project.dir);
    let mut report = crate::status::status(project, &url, STACK_TIMEOUT, running, token.as_deref());
    // The same diagnosis `chaps status` makes: when the API does not answer,
    // its container has been saying why in its own log.
    if !matches!(report.api, ApiHealth::Up { .. })
        && let Some(containers) = containers
    {
        report.unhealthy =
            crate::diagnose::failing_containers(project, containers, Some(API_SERVICE));
    }
    Check::from_verdict(ID, NAME, stack_verdict(&report))
}

// ---------------------------------------------------------------------------
// Running an external command with a deadline
// ---------------------------------------------------------------------------

/// What came of running one external command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It ran to completion, for better or worse.
    Done {
        ok: bool,
        stdout: String,
        stderr: String,
    },
    /// The binary is not on PATH.
    Missing,
    /// It was still running when its deadline passed, and was killed.
    TimedOut,
    /// It could not be started for some other reason.
    Failed(String),
}

impl Outcome {
    /// Whether the command ran and exited zero.
    pub fn succeeded(&self) -> bool {
        matches!(self, Outcome::Done { ok: true, .. })
    }

    /// What the command printed on stdout, empty when it never ran.
    pub fn stdout(&self) -> &str {
        match self {
            Outcome::Done { stdout, .. } => stdout,
            _ => "",
        }
    }

    /// Whatever the command said about its own failure.
    pub fn stderr(&self) -> &str {
        match self {
            Outcome::Done { stderr, .. } => stderr,
            Outcome::Failed(why) => why,
            Outcome::Missing => "the binary is not on PATH",
            Outcome::TimedOut => "it did not finish in time",
        }
    }
}

/// Run `program args...` and give up on it after `timeout`.
///
/// `std::process` has no deadline of its own, so the child is polled and
/// killed rather than waited on: a wedged daemon must not be able to hold
/// `chaps doctor` open. Every output here is small enough to sit in the pipe
/// buffer while the child is polled, so nothing can deadlock on an unread
/// pipe.
pub fn run_bounded(program: &str, args: &[&str], timeout: Duration) -> Outcome {
    crate::output::verbose(&format!("$ {program} {}", args.join(" ")));
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Outcome::Missing,
        Err(err) => return Outcome::Failed(format!("could not run `{program}`: {err}")),
    };

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return match child.wait_with_output() {
                    Ok(output) => Outcome::Done {
                        ok: status.success(),
                        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
                        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                    },
                    Err(err) => Outcome::Failed(format!("reading from `{program}`: {err}")),
                };
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Outcome::TimedOut;
                }
                std::thread::sleep(POLL);
            }
            Err(err) => return Outcome::Failed(format!("waiting for `{program}`: {err}")),
        }
    }
}

/// The first non-empty line of some output, short enough for one column.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string();
    if line.chars().count() <= 120 {
        return line;
    }
    format!("{}...", line.chars().take(117).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{ApiVersion, ModelStatus};
    use std::collections::BTreeMap;

    /// The tag the `user` checks are asked about in these tests.
    const IMAGE_REF: &str = "ghcr.io/chap-models/chapkit_ewars_model:sha-fa880a1";

    /// A finished report, so the rendering and the counting can be driven
    /// without anything having been probed.
    fn report(checks: Vec<Check>, project: Option<&str>) -> Report {
        Report {
            summary: summary(&checks),
            checks,
            project: project.map(PathBuf::from),
        }
    }

    /// A model this deployment added itself, following `main` at `tag`.
    fn manual_entry(tag: &str, follow: Option<&str>) -> crate::project::ManualModel {
        crate::project::ManualModel {
            service_id: "chapkit-ghr-model".into(),
            display_name: "chapkit_ghr_model".into(),
            repository: Some("https://github.com/chap-models/chapkit_ghr_model".into()),
            image: "ghcr.io/chap-models/chapkit_ghr_model".into(),
            tag: tag.to_string(),
            commit: None,
            follow: follow.map(str::to_string),
            data_dir: Some("/work/data".into()),
            user: Some("10001:10001".into()),
            runtime_amd64: true,
            added: "2026-09-24".into(),
        }
    }

    /// The `manual <id>` line: a warning only when the branch has moved on,
    /// and never a failure.
    #[test]
    fn a_manual_model_is_checked_against_the_branch_it_follows() {
        let entry = manual_entry("sha-1eb8cf1", Some("main"));

        let ok = manual_check("chapkit_ghr_model", &entry, Some("sha-1eb8cf1"), false);
        assert_eq!(ok.status, Status::Ok);
        assert_eq!(ok.id, "manual-chapkit_ghr_model");
        assert_eq!(ok.name, "manual chapkit_ghr_model");
        assert!(ok.detail.contains("the newest published build on main"));
        assert_eq!(ok.fix, None);

        let behind = manual_check("chapkit_ghr_model", &entry, Some("sha-b1d6c31"), false);
        assert_eq!(behind.status, Status::Warn);
        assert!(behind.detail.contains("running sha-1eb8cf1"), "{behind:?}");
        assert!(
            behind.detail.contains("published sha-b1d6c31"),
            "{behind:?}"
        );
        assert_eq!(
            behind.fix.as_deref(),
            Some("run `chaps update` to move the pin")
        );

        // A repository that would not answer is a check that was not made.
        let unknown = manual_check("chapkit_ghr_model", &entry, None, false);
        assert_eq!(unknown.status, Status::Skip);
        assert!(unknown.detail.contains("could not ask"), "{unknown:?}");
        assert!(unknown.fix.is_some());

        // `--offline` asks nothing, and an entry with no branch has nothing
        // to be behind.
        let offline = manual_check("chapkit_ghr_model", &entry, None, true);
        assert_eq!(offline.status, Status::Skip);
        assert!(offline.detail.contains("--offline"), "{offline:?}");

        let pinned = manual_check(
            "chapkit_ghr_model",
            &manual_entry("sha-1eb8cf1", None),
            Some("sha-b1d6c31"),
            false,
        );
        assert_eq!(pinned.status, Status::Skip);
        assert!(
            pinned.detail.contains("pinned to sha-1eb8cf1"),
            "{pinned:?}"
        );
    }

    /// The `user <service>` line: what the overlay runs the model as, against
    /// what the pulled image says it runs as.
    #[test]
    fn a_models_user_is_checked_against_the_image_it_pins() {
        // The image declares `USER chapkit`; the entry records the numbers
        // that resolves to, so the two agree.
        let ok = user_check(
            "chapkit_ewars_model",
            "chapkit-ewars-model",
            Some(IMAGE_REF),
            "1000:1000",
            Some("chapkit"),
        );
        assert_eq!(ok.status, Status::Ok);
        assert_eq!(ok.id, "user-chapkit-ewars-model");
        assert_eq!(ok.name, "user chapkit-ewars-model");
        assert!(ok.detail.contains("as the image declares"), "{ok:?}");
        assert_eq!(ok.fix, None);

        // Root, however it is spelled on either side. An empty `User` is one
        // of those spellings: the parser has already ruled out the empty
        // config that only looks like root.
        for declared in ["root", "", "0:0"] {
            let check = user_check("m", "m", Some(IMAGE_REF), "root", Some(declared));
            assert_eq!(check.status, Status::Ok, "{declared:?} {check:?}");
        }

        // The bug this check exists for: an image that runs as root under an
        // overlay that hands it an unprivileged uid.
        let wrong = user_check(
            "chapkit_rwanda_malaria_bym_model",
            "chapkit-rwanda-malaria-bym-model",
            Some(IMAGE_REF),
            "1000:1000",
            Some("root"),
        );
        assert_eq!(wrong.status, Status::Warn);
        assert!(
            wrong
                .detail
                .contains("1000:1000, but the image runs as root"),
            "{wrong:?}"
        );
        assert_eq!(
            wrong.fix.as_deref(),
            Some(
                "run `chaps models enable chapkit_rwanda_malaria_bym_model` to read the \
                 user off the image again, or `chaps update`"
            )
        );

        // Nothing to ask: the amd64 variant is not in the image store, which
        // is also what an arm64 host that has pulled nothing yet looks like.
        let absent = user_check("m", "m", Some(IMAGE_REF), "1000:1000", None);
        assert_eq!(absent.status, Status::Skip);
        assert!(
            absent
                .detail
                .contains(&format!("the amd64 variant of {IMAGE_REF} is not in the")),
            "{absent:?}"
        );
        assert_eq!(
            absent.fix.as_deref(),
            Some(
                "run `chaps docker pull`, or `docker pull --platform linux/amd64 \
                 ghcr.io/chap-models/chapkit_ewars_model:sha-fa880a1`"
            )
        );

        // No docker CLI at all is a different reason, and no pull would fix
        // it.
        let no_cli = user_check("m", "m", None, "1000:1000", None);
        assert_eq!(no_cli.status, Status::Skip);
        assert!(no_cli.detail.contains("no docker CLI"), "{no_cli:?}");
        assert_eq!(no_cli.fix, None);

        // And an account name only the image can resolve proves nothing about
        // the numbers recorded beside it.
        let opaque = user_check("m", "m", Some(IMAGE_REF), "10001:10001", Some("app"));
        assert_eq!(opaque.status, Status::Skip);
        assert!(opaque.detail.contains("`app`"), "{opaque:?}");
    }

    /// One line per enabled model, and none at all without a docker CLI to
    /// ask.
    #[test]
    fn the_user_checks_cover_every_enabled_model() {
        let entry = crate::project::EnabledModel {
            service_id: "chapkit-ewars-model".into(),
            image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
            image_tag: "sha-fa880a1".into(),
            version: "1.0.0".into(),
            channel: None,
            host_port: None,
            data_dir: "/app/data".into(),
            user: "1000:1000".into(),
            user_from: Default::default(),
            platform: None,
            compose_file: "compose.chapkit-ewars-model.yml".into(),
        };
        let mut project = Project {
            dir: std::path::PathBuf::from("/tmp/chapx"),
            state: crate::project::ProjectState {
                models: std::collections::BTreeMap::from([(
                    "chapkit_ewars_model".to_string(),
                    entry,
                )]),
                ..crate::project::ProjectState::default()
            },
        };
        let checks = user_checks_with(&project, true, &|reference| {
            reference
                .contains("ewars")
                .then(|| ("root".to_string(), "/app".to_string()))
        });
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].id, "user-chapkit-ewars-model");
        assert_eq!(checks[0].status, Status::Warn);

        // Without a docker CLI nothing is asked, but the line is still there.
        let skipped = user_checks_with(&project, false, &|_| panic!("no CLI, no question"));
        assert_eq!(skipped[0].status, Status::Skip);
        assert!(skipped[0].detail.contains("no docker CLI"), "{skipped:?}");

        // A daemon that cannot answer for the amd64 variant - an arm64 host
        // that has pulled nothing, or has pulled only its own architecture -
        // names the pull that would let the line be answered.
        let unknown = user_checks_with(&project, true, &|_| None);
        assert_eq!(unknown[0].status, Status::Skip);
        assert!(
            unknown[0].detail.contains(
                "the amd64 variant of ghcr.io/chap-models/chapkit_ewars_model:sha-fa880a1"
            ),
            "{unknown:?}"
        );
        assert!(
            unknown[0]
                .fix
                .as_deref()
                .is_some_and(|fix| fix.contains("chaps docker pull")),
            "{unknown:?}"
        );

        // A deployment with no models has no such lines.
        project.state.models.clear();
        assert!(user_checks_with(&project, true, &|_| None).is_empty());
    }

    /// What `docker version` prints when it worked.
    fn done(ok: bool, stdout: &str, stderr: &str) -> Outcome {
        Outcome::Done {
            ok,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn the_columns_line_up_whatever_the_names_are() {
        let report = report(
            vec![
                Check::ok("docker-cli", "docker cli", "docker 29.8.1"),
                Check::warn("disk", "disk", "1.0 GB free", "free up space"),
                Check::fail("api-port", "api port", "8000 is in use", "free it"),
            ],
            Some("/srv/mychap"),
        );
        assert_eq!(
            render(&report, &Out::default()),
            "ok    docker cli  docker 29.8.1\n\
             warn  disk        1.0 GB free\n      \
             free up space\n\
             fail  api port    8000 is in use\n      \
             free it\n\
             \n\
             3 checks: 1 ok, 1 warn, 1 fail\n"
        );
    }

    #[test]
    fn without_a_project_the_rendering_says_where_the_rest_would_be() {
        let outside = report(vec![Check::ok("chaps", "chaps", "v0.2.0")], None);
        let text = render(&outside, &Out::default());
        assert!(text.contains(NO_PROJECT), "{text}");
        assert!(text.ends_with("1 checks: 1 ok, 0 warn, 0 fail\n"), "{text}");

        // Inside one, the line is not printed at all.
        let inside = report(vec![Check::ok("chaps", "chaps", "v0.2.0")], Some("/srv/x"));
        assert!(!render(&inside, &Out::default()).contains("none here"));
    }

    #[test]
    fn every_status_word_fits_the_column() {
        for status in [Status::Ok, Status::Warn, Status::Fail, Status::Skip] {
            assert!(
                status.word().len() <= STATUS_WIDTH,
                "`{}` does not fit",
                status.word()
            );
        }
    }

    #[test]
    fn the_summary_counts_each_status_and_only_a_failure_exits_non_zero() {
        let checks = vec![
            Check::ok("a", "a", ""),
            Check::ok("b", "b", ""),
            Check::warn("c", "c", "", "do something"),
            Check::skip("d", "d", ""),
        ];
        let counted = summary(&checks);
        assert_eq!(
            counted,
            Summary {
                ok: 2,
                warn: 1,
                fail: 0,
                skip: 1
            }
        );
        assert_eq!(exit_code(&counted), 0, "a warning is not a failure");
        assert_eq!(
            summary_line(&counted),
            "4 checks: 2 ok, 1 warn, 0 fail, 1 skipped"
        );

        let mut failed = checks;
        failed.push(Check::fail("e", "e", "", "fix it"));
        let counted = summary(&failed);
        assert_eq!(counted.fail, 1);
        assert_eq!(exit_code(&counted), 1);

        // With nothing skipped the sentence is the three words that matter.
        let counted = summary(&[Check::ok("a", "a", "")]);
        assert_eq!(summary_line(&counted), "1 checks: 1 ok, 0 warn, 0 fail");
    }

    #[test]
    fn json_is_checks_and_a_summary_and_nothing_else() {
        let report = report(
            vec![
                Check::ok("docker-cli", "docker cli", "docker 29.8.1"),
                Check::warn("disk", "disk space", "4.0 GB free", "free up space"),
            ],
            Some("/srv/mychap"),
        );
        let value: serde_json::Value = serde_json::to_value(&report).unwrap();
        let object = value.as_object().expect("an object");
        assert_eq!(
            object.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["checks", "summary"],
            "the project path is not part of the document"
        );
        assert_eq!(
            value["checks"][0],
            serde_json::json!({
                "id": "docker-cli",
                "name": "docker cli",
                "status": "ok",
                "detail": "docker 29.8.1",
                "fix": null,
            })
        );
        assert_eq!(value["checks"][1]["status"], "warn");
        assert_eq!(value["checks"][1]["fix"], "free up space");
        assert_eq!(
            value["summary"],
            serde_json::json!({"ok": 1, "warn": 1, "fail": 0, "skip": 0})
        );
    }

    #[test]
    fn a_missing_docker_binary_says_where_to_get_one() {
        let check = docker_cli_check(&Outcome::Missing);
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("not on PATH"), "{}", check.detail);
        assert!(
            check
                .fix
                .unwrap()
                .contains("https://docs.docker.com/get-docker/"),
            "the fix has to carry the link"
        );

        let check = docker_cli_check(&done(true, "29.8.1", ""));
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.detail, "docker 29.8.1");

        assert_eq!(docker_cli_check(&Outcome::TimedOut).status, Status::Fail);
    }

    #[test]
    fn the_daemon_error_is_told_apart_by_what_docker_printed() {
        // The two messages a real docker prints, verbatim.
        assert_eq!(
            daemon_problem(
                "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. \
                 Is the docker daemon running?"
            ),
            DaemonProblem::NotRunning
        );
        assert_eq!(
            daemon_problem(
                "Got permission denied while trying to connect to the Docker daemon socket at \
                 unix:///var/run/docker.sock: Get \"http://%2Fvar%2Frun%2Fdocker.sock/v1.24/info\": \
                 dial unix /var/run/docker.sock: connect: permission denied"
            ),
            DaemonProblem::PermissionDenied,
            "permission denied is not `the daemon is not there`"
        );
        // Windows spells the same absence differently.
        assert_eq!(
            daemon_problem(
                "error during connect: Get \"http://%2F%2F.%2Fpipe%2FdockerDesktopLinuxEngine\
                 /v1.51/info\": open //./pipe/dockerDesktopLinuxEngine: The system cannot find \
                 the file specified."
            ),
            DaemonProblem::NotRunning
        );
        assert_eq!(
            daemon_problem("something else entirely"),
            DaemonProblem::Unknown
        );

        assert!(daemon_fix(DaemonProblem::NotRunning).contains("systemctl start docker"));
        assert!(daemon_fix(DaemonProblem::PermissionDenied).contains("docker` group"));
    }

    #[test]
    fn the_daemon_check_reports_the_cause_it_found() {
        let check = docker_daemon_check(
            &done(
                false,
                "",
                "Cannot connect to the Docker daemon at unix:///var/run/docker.sock",
            ),
            true,
        );
        assert_eq!(check.status, Status::Fail);
        assert_eq!(check.detail, "the Docker daemon is not running");

        let check = docker_daemon_check(&done(false, "", "permission denied"), true);
        assert_eq!(check.detail, "permission denied on the Docker socket");

        let check = docker_daemon_check(&done(true, "29.8.0", ""), true);
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.detail, "Docker Engine 29.8.0");

        // Nothing to ask when the CLI itself is missing; the line above said so.
        assert_eq!(
            docker_daemon_check(&Outcome::Missing, false).status,
            Status::Skip
        );
    }

    /// The line follows [`docker::MIN_COMPOSE_VERSION`], which is the release
    /// `!override` arrived in: 2.24.3 has `include:` and still cannot run the
    /// port override every deployment gets, so it is a warning and 2.24.4 is
    /// not.
    #[test]
    fn compose_warns_below_the_version_the_override_tag_needs() {
        let (status, detail, fix) = compose_verdict(Some((2, 24, 3)));
        assert_eq!(status, Status::Warn);
        assert_eq!(detail, "v2.24.3, older than 2.24.4");
        let fix = fix.expect("a warning says what to do");
        assert!(fix.contains("`!override`"), "{fix}");
        assert!(fix.contains("include:"), "{fix}");

        let (status, detail, fix) = compose_verdict(Some((2, 24, 4)));
        assert_eq!(status, Status::Ok);
        assert_eq!(detail, "v2.24.4");
        assert_eq!(fix, None);

        assert_eq!(compose_verdict(Some((5, 5, 1))).0, Status::Ok);
        assert_eq!(compose_verdict(Some((5, 5, 1))).1, "v5.5.1");
        // Old enough to lose the model overlays as well as the port override.
        assert_eq!(compose_verdict(Some((2, 20, 0))).0, Status::Warn);

        let (status, detail, fix) = compose_verdict(Some((2, 19, 9)));
        assert_eq!(status, Status::Warn);
        assert_eq!(detail, "v2.19.9, older than 2.24.4");
        assert!(fix.unwrap().contains("include:"));

        let (status, detail, fix) = compose_verdict(None);
        assert_eq!(status, Status::Fail);
        assert_eq!(detail, "docker compose is not installed");
        assert!(fix.unwrap().contains("2.24.4 or newer"));

        // And with no docker at all there is nothing to report.
        assert_eq!(compose_check(false, None).status, Status::Skip);
        assert_eq!(compose_check(true, Some((2, 24, 6))).status, Status::Ok);
    }

    #[test]
    fn arm_hosts_are_warned_that_the_images_are_amd64() {
        let check = arch_check("linux", "x86_64");
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.detail, "linux/x86_64");
        assert_eq!(check.fix, None);

        let check = arch_check("macos", "aarch64");
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("linux/amd64"), "{}", check.detail);
        assert!(check.fix.unwrap().contains("Rosetta"));

        let check = arch_check("linux", "aarch64");
        assert_eq!(check.status, Status::Warn);
        assert!(check.fix.unwrap().contains("binfmt"));

        // Windows spells the same architecture differently.
        assert_eq!(arch_check("windows", "arm64").status, Status::Warn);
    }

    #[test]
    fn docker_info_is_asked_once_and_split_afterwards() {
        assert_eq!(
            info_fields("29.8.0\t/var/lib/docker"),
            ("29.8.0".to_string(), Some("/var/lib/docker".to_string()))
        );
        // A field the engine does not know comes back empty, not absent.
        assert_eq!(info_fields("29.8.0\t"), ("29.8.0".to_string(), None));
        assert_eq!(info_fields("29.8.0"), ("29.8.0".to_string(), None));
        assert_eq!(info_fields(""), (String::new(), None));
    }

    #[test]
    fn the_measured_filesystem_falls_back_when_the_root_is_not_out_here() {
        let here = std::env::current_dir().expect("a working directory");
        // Docker Desktop's root lives inside its VM, so it is not this host's.
        let inside_the_vm = if cfg!(windows) {
            "C:\\definitely\\not\\a\\directory"
        } else {
            "/definitely/not/a/directory"
        };
        assert_eq!(disk_path(Some(inside_the_vm)), here);
        assert_eq!(disk_path(None), here);

        // A root this host can see is the one to measure.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().to_string_lossy().into_owned();
        assert_eq!(disk_path(Some(&path)), PathBuf::from(&path));
    }

    #[test]
    fn the_disk_thresholds_are_ten_and_three_gigabytes() {
        assert_eq!(disk_verdict(Some(64 * GB)).0, Status::Ok);
        assert_eq!(disk_verdict(Some(DISK_WARN)).0, Status::Ok);
        assert_eq!(disk_verdict(Some(DISK_WARN - 1)).0, Status::Warn);
        assert_eq!(disk_verdict(Some(DISK_FAIL)).0, Status::Warn);
        assert_eq!(disk_verdict(Some(DISK_FAIL - 1)).0, Status::Fail);
        assert_eq!(disk_verdict(Some(0)).0, Status::Fail);
        // Unparsable is not "no space": it is a question this host did not
        // answer, and a failure would be a lie.
        assert_eq!(disk_verdict(None).0, Status::Skip);

        assert_eq!(disk_verdict(Some(4 * GB)).1, "4.0 GB free");
        let check = disk_check(Path::new("/var/lib/docker"), Some(4 * GB));
        assert_eq!(check.detail, "4.0 GB free on /var/lib/docker");
        // A skipped check must not claim to have measured anything.
        assert!(!disk_check(Path::new("/x"), None).detail.contains("/x"));
    }

    #[test]
    fn free_space_is_read_out_of_what_each_platform_prints() {
        // macOS `df -Pk /`, which is the same shape as Linux's.
        let macos = "Filesystem 1024-blocks      Used Available Capacity  Mounted on\n\
                     /dev/disk3s1s1 971350180 21495384 663797352     4%    /\n";
        assert_eq!(parse_df(macos), Some(663_797_352 * 1024));

        // Linux, where a long device name is what `-P` keeps on one line.
        let linux = "Filesystem     1024-blocks     Used Available Capacity Mounted on\n\
                     /dev/mapper/vg0-root  103081248 40209032  57596756      42% /\n";
        assert_eq!(parse_df(linux), Some(57_596_756 * 1024));

        assert_eq!(parse_df(""), None);
        assert_eq!(parse_df("df: /nope: No such file or directory\n"), None);

        // PowerShell's `(Get-PSDrive -Name C).Free` is the number alone.
        assert_eq!(parse_free_bytes("\r\n123456789\r\n"), Some(123_456_789));
        assert_eq!(parse_free_bytes(""), None);
        assert_eq!(parse_free_bytes("Get-PSDrive : Cannot find drive"), None);
    }

    #[test]
    fn a_probe_that_did_not_answer_is_a_warning_and_offline_is_a_skip() {
        let checks = network_checks(None);
        assert_eq!(checks.len(), 3);
        for check in &checks {
            assert_eq!(check.status, Status::Skip);
            assert!(check.detail.contains("offline"), "{}", check.detail);
        }
        assert_eq!(
            checks.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["net-ghcr", "net-marketplace", "net-releases"]
        );

        let probed = Probed {
            ghcr: Ok(401),
            marketplace: Err("dns error".to_string()),
            chap_core: Ok("v2.3.1".to_string()),
            chaps: Ok("v0.2.0".to_string()),
        };
        let checks = network_checks(Some(&probed));
        assert_eq!(checks[0].status, Status::Ok);
        assert_eq!(checks[0].detail, "reachable (HTTP 401)");
        assert_eq!(checks[1].status, Status::Warn, "unreachable never fails");
        assert!(checks[1].detail.contains("dns error"));
        assert!(checks[1].fix.as_ref().unwrap().contains("cache"));
        assert_eq!(checks[2].status, Status::Ok);
        assert!(checks[2].detail.contains("v2.3.1"));
        assert!(checks[0].fix.is_none());

        // ghcr is the one whose absence stops images being pulled.
        let probed = Probed {
            ghcr: Err("timed out".to_string()),
            ..probed
        };
        let checks = network_checks(Some(&probed));
        assert_eq!(checks[0].status, Status::Warn);
        assert!(checks[0].fix.as_ref().unwrap().contains("--offline"));
    }

    #[test]
    fn the_chaps_line_offers_an_update_only_when_there_is_one() {
        // Offline the lookup never ran, so the line is a skip that says why
        // and still describes the build.
        let check = chaps_check(ReleaseList::Offline);
        assert_eq!(check.status, Status::Skip);
        assert!(check.detail.contains(VERSION) && check.detail.contains(TARGET));
        assert!(check.detail.contains("offline"), "{}", check.detail);

        // The release feed was asked and said nothing useful: still just a
        // description, and `net-releases` is where the network is reported.
        let check = chaps_check(ReleaseList::Unreachable);
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains(VERSION) && check.detail.contains(TARGET));
        let current = format!("v{VERSION}");
        assert_eq!(
            chaps_check(ReleaseList::Newest(&current)).status,
            Status::Ok,
            "the running version is not an update"
        );

        let check = chaps_check(ReleaseList::Newest("v999.0.0"));
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("v999.0.0"));
        assert_eq!(check.fix.unwrap(), "run `chaps self update`");
    }

    /// The facts of a deployment that has its credentials and no plugins: the
    /// quiet case, where the line says nothing beyond the config file.
    fn facts(config: Option<&str>) -> OcsFacts<'_> {
        OcsFacts {
            config,
            credentials: true,
            plugins: None,
            datasets: None,
            data_bytes: None,
        }
    }

    #[test]
    fn the_components_line_reports_the_set_and_the_ocs_config() {
        // Nothing but chap-core: there is no config file to have an opinion on.
        let (status, detail, fix) = components_verdict(&Components::default(), &facts(None));
        assert_eq!(status, Status::Ok);
        assert_eq!(detail, "chap-core");
        assert_eq!(fix, None);

        let mut components = Components::default();
        components.set_enabled(crate::components::Component::Ocs, true);

        // The component is on and its instance config is gone: the container
        // would start with nothing to be an instance of.
        let (status, detail, fix) = components_verdict(&components, &facts(None));
        assert_eq!(status, Status::Fail);
        assert!(
            detail.contains("ocs/climate-service.yaml is missing"),
            "{detail}"
        );
        assert!(fix.unwrap().contains("chaps sync"));

        // Present, but still the example: it deploys, it just deploys the
        // wrong country.
        let example = crate::compose::render::render_ocs_config(
            &crate::compose::spec::OcsConfigSpec::default(),
        );
        let (status, detail, fix) = components_verdict(&components, &facts(Some(&example)));
        assert_eq!(status, Status::Warn);
        assert!(detail.starts_with("chap-core, ocs; "), "{detail}");
        assert!(detail.contains("example values"), "{detail}");
        assert!(fix.unwrap().contains("--ocs-country"));

        // Edited, note deleted: nothing left to say.
        let (status, detail, fix) = components_verdict(&components, &facts(Some("id: mine\n")));
        assert_eq!(status, Status::Ok);
        assert!(
            detail.ends_with("ocs/climate-service.yaml present"),
            "{detail}"
        );
        assert_eq!(fix, None);
    }

    /// Two things the line reports and never judges: missing credentials, which
    /// are optional, and the plugin count, which is a fact about a directory
    /// the operator put there.
    #[test]
    fn the_components_line_notes_the_credentials_and_the_plugins_without_complaining() {
        let mut components = Components::default();
        components.set_enabled(crate::components::Component::Ocs, true);

        let (status, detail, fix) = components_verdict(
            &components,
            &OcsFacts {
                config: Some("id: mine\n"),
                credentials: false,
                plugins: Some(2),
                ..OcsFacts::default()
            },
        );
        assert_eq!(status, Status::Ok, "neither is a problem: {detail}");
        assert_eq!(fix, None);
        assert!(
            detail
                .contains("ERA5-Land: credentials unset (WorldPop and CHIRPS3 work without them)"),
            "{detail}"
        );
        assert!(detail.ends_with("plugins/: 2 files"), "{detail}");

        // One file is singular, and a set credential says nothing at all.
        let (_, detail, _) = components_verdict(
            &components,
            &OcsFacts {
                config: Some("id: mine\n"),
                credentials: true,
                plugins: Some(1),
                ..OcsFacts::default()
            },
        );
        assert!(!detail.contains("credentials unset"), "{detail}");
        assert!(detail.ends_with("plugins/: 1 file"), "{detail}");

        // An example config is a warning either way, and the notes wait for
        // the file to be the operator's own: one thing to fix at a time.
        let example = crate::compose::render::render_ocs_config(
            &crate::compose::spec::OcsConfigSpec::default(),
        );
        let (status, detail, _) = components_verdict(
            &components,
            &OcsFacts {
                config: Some(&example),
                credentials: false,
                plugins: Some(3),
                ..OcsFacts::default()
            },
        );
        assert_eq!(status, Status::Warn);
        assert!(!detail.contains("plugins/"), "{detail}");
    }

    /// The same two facts `chaps status` puts on the OCS line: what the
    /// instance holds, reported and never judged.
    #[test]
    fn the_components_line_reports_what_a_running_ocs_holds() {
        let mut components = Components::default();
        components.set_enabled(crate::components::Component::Ocs, true);

        let (status, detail, fix) = components_verdict(
            &components,
            &OcsFacts {
                config: Some("id: mine\n"),
                credentials: true,
                datasets: Some(3),
                data_bytes: Some(217_088 * 1024),
                ..OcsFacts::default()
            },
        );
        assert_eq!(status, Status::Ok, "neither is a problem: {detail}");
        assert_eq!(fix, None);
        assert_eq!(
            detail,
            "chap-core, ocs; ocs/climate-service.yaml present; 3 datasets; 212.0 MB data"
        );

        // One dataset is singular, and an instance that is not running, or
        // did not answer, says neither thing rather than saying nothing.
        let (_, detail, _) = components_verdict(
            &components,
            &OcsFacts {
                config: Some("id: mine\n"),
                credentials: true,
                datasets: Some(1),
                ..OcsFacts::default()
            },
        );
        assert!(detail.ends_with("; 1 dataset"), "{detail}");
        let (_, detail, _) = components_verdict(
            &components,
            &OcsFacts {
                config: Some("id: mine\n"),
                credentials: true,
                ..OcsFacts::default()
            },
        );
        assert!(detail.ends_with("present"), "{detail}");
    }

    /// Files at any depth, because OCS's own layout is `plugins/datasets/*.py`.
    #[test]
    fn the_plugin_count_reaches_into_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        assert_eq!(plugin_count(&plugins), None, "no directory, no count");

        std::fs::create_dir_all(plugins.join("datasets")).unwrap();
        assert_eq!(plugin_count(&plugins), Some(0));
        std::fs::write(plugins.join("datasets").join("clms_gpp.py"), "").unwrap();
        std::fs::write(plugins.join("datasets").join("clms_gpp.yaml"), "").unwrap();
        std::fs::write(plugins.join("README.md"), "").unwrap();
        assert_eq!(plugin_count(&plugins), Some(3));
    }

    #[test]
    fn the_component_images_are_the_ones_the_enabled_set_pulls() {
        assert!(component_images(&Components::default()).is_empty());

        let mut components = Components::default();
        components.set_enabled(crate::components::Component::Ocs, true);
        components.set_enabled(crate::components::Component::S3, true);
        assert_eq!(
            component_images(&components),
            vec![
                (
                    "ocs".to_string(),
                    "ghcr.io/dhis2/open-climate-service:main".to_string()
                ),
                ("s3".to_string(), "rustfs/rustfs:latest".to_string()),
            ]
        );
    }

    #[test]
    fn a_component_image_is_checked_for_existence_not_for_amd64() {
        // Both component images are multi-arch and no overlay pins a platform,
        // so a manifest that answers at all is the whole verdict.
        let arm_only = r#"{"manifests":[{"platform":{"os":"linux","architecture":"arm64"}}]}"#;
        let check = component_image_verdict(
            "ocs",
            "ghcr.io/dhis2/open-climate-service:main",
            &Outcome::Done {
                ok: true,
                stdout: arm_only.to_string(),
                stderr: String::new(),
            },
        );
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.id, "image-ocs");

        let missing = component_image_verdict(
            "ocs",
            "ghcr.io/dhis2/open-climate-service:nope",
            &Outcome::Done {
                ok: false,
                stdout: String::new(),
                stderr: "manifest unknown".to_string(),
            },
        );
        assert_eq!(missing.status, Status::Fail);
        assert!(missing.detail.contains("manifest unknown"));
        assert!(missing.fix.unwrap().contains("OCS_IMAGE_TAG"));

        assert_eq!(
            component_image_verdict("s3", "rustfs/rustfs:latest", &Outcome::Missing).status,
            Status::Skip
        );
    }

    #[test]
    fn the_stack_line_judges_a_deployment_without_chap_core_by_its_components() {
        use crate::status::{ComponentState, ComponentStatus};

        let component = |name: &str, state: ComponentState| ComponentStatus {
            name: name.to_string(),
            state,
            reach: "http://localhost:9000".to_string(),
            health_url: None,
            read_only: false,
            datasets: None,
            data_bytes: None,
        };
        let mut report = status_report(ApiHealth::Off, &[], &[]);
        report.components = vec![component("ocs", ComponentState::Up)];
        let (status, detail, fix) = stack_verdict(&report);
        assert_eq!(status, Status::Ok);
        assert_eq!(detail, "no chap-core here; up: ocs");
        assert_eq!(fix, None);

        report.components = vec![
            component("ocs", ComponentState::Starting),
            component("s3", ComponentState::Up),
        ];
        let (status, detail, fix) = stack_verdict(&report);
        assert_eq!(status, Status::Warn);
        assert_eq!(detail, "no chap-core here; not up: ocs");
        assert!(fix.unwrap().contains("chaps status"));
    }

    #[test]
    fn the_file_list_is_what_init_writes() {
        let files = project_files(&Components::default());
        assert_eq!(files.len(), 7);
        for name in [
            ".chaps/project.yaml",
            ".chaps/models.yaml",
            ".chaps/components.yaml",
            ".env",
            "compose.yml",
            "compose.chaps.yml",
            "compose.marketplace.yml",
        ] {
            assert!(files.contains(&name.to_string()), "{name} is missing");
        }

        // A component adds its own compose file, between the override and the
        // umbrella; chap-core off takes the base stack out of the list.
        let mut components = Components::default();
        components.set_enabled(crate::components::Component::Ocs, true);
        assert_eq!(
            project_files(&components),
            vec![
                ".chaps/project.yaml",
                ".chaps/models.yaml",
                ".chaps/components.yaml",
                ".env",
                "compose.yml",
                "compose.chaps.yml",
                "compose.ocs.yml",
                "compose.marketplace.yml",
            ]
        );
        components.set_enabled(crate::components::Component::ChapCore, false);
        assert_eq!(
            project_files(&components),
            vec![
                ".chaps/project.yaml",
                ".chaps/models.yaml",
                ".chaps/components.yaml",
                ".env",
                "compose.ocs.yml",
                "compose.marketplace.yml",
            ]
        );

        let (status, detail, fix) = files_verdict(6, &[]);
        assert_eq!(status, Status::Ok);
        assert_eq!(detail, "all 6 present");
        assert_eq!(fix, None);

        let (status, detail, fix) =
            files_verdict(6, &[".env".to_string(), "compose.yml".to_string()]);
        assert_eq!(status, Status::Fail);
        assert_eq!(detail, "missing .env, compose.yml");
        let fix = fix.unwrap();
        assert!(fix.contains("chaps sync") && fix.contains("chaps init --force"));
    }

    #[test]
    fn the_env_check_reads_the_two_things_that_bite_later() {
        // Nothing to read is not a fault: `project-files` already said so.
        assert_eq!(env_verdict(None, None).0, Status::Skip);

        let good = "POSTGRES_PASSWORD=0123456789abcdef\n\
                    CHAP_API_TOKEN=sekret\n\
                    # CHAP_IMAGE_TAG=latest\n";
        let (status, detail, fix) = env_verdict(Some(good), Some(OVERRIDE));
        assert_eq!(status, Status::Ok);
        assert_eq!(
            detail,
            "auth on, POSTGRES_PASSWORD set, CHAP_IMAGE_TAG present"
        );
        assert_eq!(fix, None);

        // Authentication off is reported, not complained about.
        let open = "POSTGRES_PASSWORD=0123456789abcdef\n# CHAP_API_TOKEN=\nCHAP_IMAGE_TAG=v2.3.1\n";
        let (status, detail, _) = env_verdict(Some(open), Some(OVERRIDE));
        assert_eq!(status, Status::Ok);
        assert!(detail.starts_with("auth off, "), "{detail}");

        // The default password is the one that has to be moved off.
        let (status, detail, fix) = env_verdict(
            Some("POSTGRES_PASSWORD=chap\n# CHAP_IMAGE_TAG=latest\n"),
            None,
        );
        assert_eq!(status, Status::Warn);
        assert!(detail.contains("the default `chap`"), "{detail}");
        assert!(fix.unwrap().contains("ALTER USER"));

        // An unset one resolves to the same default through compose.
        let (status, detail, _) = env_verdict(Some("# CHAP_IMAGE_TAG=latest\n"), None);
        assert_eq!(status, Status::Warn);
        assert!(detail.contains("unset"), "{detail}");

        // And a file the pin comments have been cut out of.
        let (status, detail, fix) = env_verdict(Some("POSTGRES_PASSWORD=0123456789abcdef\n"), None);
        assert_eq!(status, Status::Warn);
        assert!(detail.contains("no CHAP_IMAGE_TAG line"), "{detail}");
        assert!(fix.unwrap().contains("chaps sync"));
    }

    /// A `compose.chaps.yml` as this version renders it: it hands chap-core
    /// the registration key.
    const OVERRIDE: &str = "services:\n  chap:\n    environment:\n      \
         SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}\n";

    #[test]
    fn a_protected_deployment_whose_override_drops_the_key_is_a_warning() {
        let protected = "POSTGRES_PASSWORD=0123456789abcdef\n\
                         CHAP_API_TOKEN=sekret\n\
                         # CHAP_IMAGE_TAG=latest\n";
        // What an older chaps rendered: the port override and nothing else.
        let old_override = "services:\n  chap:\n    ports: !override\n      - \"8000:8000\"\n";
        let (status, detail, fix) = env_verdict(Some(protected), Some(old_override));
        assert_eq!(status, Status::Warn);
        assert!(
            detail.contains("compose.chaps.yml does not pass SERVICEKIT_REGISTRATION_KEY"),
            "{detail}"
        );
        let fix = fix.unwrap();
        assert!(fix.contains("chaps sync"), "{fix}");
        assert!(fix.contains("401"), "{fix}");

        // The override this version renders says nothing.
        assert_eq!(env_verdict(Some(protected), Some(OVERRIDE)).0, Status::Ok);
        // Neither does a deployment with no authentication at all: there is
        // no key to hand over.
        let open = "POSTGRES_PASSWORD=0123456789abcdef\n# CHAP_IMAGE_TAG=latest\n";
        assert_eq!(env_verdict(Some(open), Some(old_override)).0, Status::Ok);
    }

    #[test]
    fn the_project_line_warns_while_the_name_is_only_the_directory() {
        // A deployment this version wrote records a name of its own.
        let (status, detail, fix) = project_name_verdict(Some("demo-1ab2c3"), "demo");
        assert_eq!(status, Status::Ok);
        assert_eq!(detail, "demo-1ab2c3 (recorded in .chaps/project.yaml)");
        assert_eq!(fix, None);

        // One written before that has the directory name, which another
        // deployment in another directory of the same name also has.
        let (status, detail, fix) = project_name_verdict(None, "demo");
        assert_eq!(status, Status::Warn);
        assert_eq!(
            detail,
            "compose project name is the directory name; volumes can collide with other \
             deployments named demo"
        );
        let fix = fix.unwrap();
        assert!(fix.contains("chaps sync"), "{fix}");
        // The fix renames nothing: it writes down the name compose already
        // uses, which is what keeps the running deployment's volumes.
        assert!(fix.contains("`demo`"), "{fix}");
        assert!(fix.contains("nothing is renamed"), "{fix}");
        assert!(
            project_name_verdict(None, "My Chap")
                .2
                .unwrap()
                .contains("`mychap`")
        );
    }

    /// The volumes of a deployment, as `docker volume ls` reports them.
    fn volumes(names: &[(&str, u64)]) -> Vec<(String, Option<u64>)> {
        names
            .iter()
            .map(|(name, at)| (name.to_string(), Some(*at)))
            .collect()
    }

    /// When `.chaps/project.yaml` was created, in these tests.
    const CREATED: u64 = 1_790_147_400;

    #[test]
    fn a_database_volume_older_than_the_deployment_is_a_warning() {
        let prefix = "demo-1ab2c3_";
        let db = "demo-1ab2c3_chap-db";

        // Nothing started yet: nothing to be suspicious of.
        let (status, detail, _) = volume_verdict(prefix, db, &[], Some(CREATED), &[], None);
        assert_eq!(status, Status::Ok);
        assert!(detail.contains("no demo-1ab2c3_* volume yet"), "{detail}");

        // Volumes this deployment made itself.
        let mine = volumes(&[(db, CREATED + 60), ("demo-1ab2c3_logs", CREATED + 60)]);
        let (status, detail, fix) = volume_verdict(prefix, db, &mine, Some(CREATED), &[], None);
        assert_eq!(status, Status::Ok);
        assert_eq!(detail, "2 volumes named demo-1ab2c3_*");
        assert_eq!(fix, None);

        // With a size for the OCS data volume, the line says how much data a
        // `down --volumes` would destroy rather than only how many volumes.
        let (status, detail, _) = volume_verdict(
            prefix,
            db,
            &mine,
            Some(CREATED),
            &[],
            Some(("demo-1ab2c3_ocs_data", 217_088 * 1024)),
        );
        assert_eq!(status, Status::Ok);
        assert_eq!(
            detail,
            "2 volumes named demo-1ab2c3_*; demo-1ab2c3_ocs_data holds 212.0 MB"
        );

        // A database volume that predates the directory it belongs to came
        // from somewhere else, and still holds that deployment's password.
        let inherited = volumes(&[(db, CREATED - 86_400), ("demo-1ab2c3_logs", CREATED + 60)]);
        let (status, detail, fix) =
            volume_verdict(prefix, db, &inherited, Some(CREATED), &[], None);
        assert_eq!(status, Status::Warn);
        assert!(
            detail.starts_with(
                "the database volume demo-1ab2c3_chap-db predates this deployment; \
                 if chap-core cannot log in, it belongs to an earlier deployment with \
                 the same name"
            ),
            "{detail}"
        );
        assert!(fix.unwrap().contains("chaps down --volumes"));

        // Neither time is guaranteed: a filesystem that records no creation
        // time, and a docker that did not say, each cost the comparison only.
        assert_eq!(
            volume_verdict(prefix, db, &inherited, None, &[], None).0,
            Status::Ok
        );
        let undated = vec![(db.to_string(), None)];
        assert_eq!(
            volume_verdict(prefix, db, &undated, Some(CREATED), &[], None).0,
            Status::Ok
        );
        // And an old volume that is not the database is not this warning.
        let other = volumes(&[("demo-1ab2c3_logs", CREATED - 86_400)]);
        assert_eq!(
            volume_verdict(prefix, db, &other, Some(CREATED), &[], None).0,
            Status::Ok
        );
    }

    /// The volume of a model or a component this deployment no longer enables
    /// is data nothing will ever mount again, and nothing else names it: the
    /// overlay that declared it is gone, so `down --volumes` cannot reach it
    /// either.
    #[test]
    fn a_volume_of_something_no_longer_enabled_is_a_warning_naming_it() {
        let prefix = "demo-1ab2c3_";
        let db = "demo-1ab2c3_chap-db";
        let ewars = "demo-1ab2c3_ck_chapkit_ewars_model_data";

        let held = volumes(&[(db, CREATED + 60), (ewars, CREATED + 60)]);
        let (status, detail, fix) = volume_verdict(
            prefix,
            db,
            &held,
            Some(CREATED),
            &[ewars.to_string(), "demo-1ab2c3_ocs_data".to_string()],
            None,
        );
        assert_eq!(status, Status::Warn);
        assert_eq!(
            detail,
            "leftover volumes from disabled models or components: \
             demo-1ab2c3_ck_chapkit_ewars_model_data, demo-1ab2c3_ocs_data"
        );
        let fix = fix.expect("a leftover volume has something to do about it");
        assert!(fix.contains("chaps models disable <id> --purge"), "{fix}");
        assert!(
            fix.contains("chaps components disable <name> --purge"),
            "{fix}"
        );
        assert!(fix.contains("docker volume rm <name>"), "{fix}");
        // `down --volumes` is the one answer that does not work here.
        assert!(!fix.contains("--volumes"), "{fix}");

        // A database volume older than the deployment is the worse of the two
        // findings, and the one the line reports.
        let inherited = volumes(&[(db, CREATED - 86_400), (ewars, CREATED + 60)]);
        let (status, detail, _) = volume_verdict(
            prefix,
            db,
            &inherited,
            Some(CREATED),
            &[ewars.to_string()],
            None,
        );
        assert_eq!(status, Status::Warn);
        assert!(detail.starts_with("the database volume"), "{detail}");

        // A leftover OCS volume is measured too: it is data nothing will
        // mount again, and its size is what decides whether to keep it.
        let (_, detail, _) = volume_verdict(
            prefix,
            db,
            &held,
            Some(CREATED),
            &["demo-1ab2c3_ocs_data".to_string()],
            Some(("demo-1ab2c3_ocs_data", 3 * 1024 * 1024 * 1024)),
        );
        assert_eq!(
            detail,
            "leftover volumes from disabled models or components: \
             demo-1ab2c3_ocs_data; demo-1ab2c3_ocs_data holds 3.0 GB"
        );
    }

    /// Which of a deployment's volumes belong to nothing it still enables.
    #[test]
    fn the_leftovers_are_the_volumes_of_disabled_models_and_components() {
        let prefix = "demo-1ab2c3_";
        let names: Vec<String> = [
            "demo-1ab2c3_chap-db",
            "demo-1ab2c3_ck_chapkit_ewars_model_data",
            "demo-1ab2c3_ck_auto_arima_chapkit_data",
            "demo-1ab2c3_ocs_data",
            "demo-1ab2c3_s3_data",
            "otherdemo_ck_chapkit_ewars_model_data",
        ]
        .iter()
        .map(|n| n.to_string())
        .collect();

        // One model enabled, no component but chap-core: everything else
        // under this prefix is a leftover, and the other deployment's volume
        // is not this deployment's business.
        let models = vec!["chapkit_ewars_model".to_string()];
        let mut components = Components::default();
        assert_eq!(
            leftover_volumes(prefix, &names, &models, &components),
            [
                "demo-1ab2c3_ck_auto_arima_chapkit_data",
                "demo-1ab2c3_ocs_data",
                "demo-1ab2c3_s3_data"
            ]
        );

        // Turning the components on leaves only the disabled model's.
        components.set_enabled(Component::Ocs, true);
        components.set_enabled(Component::S3, true);
        assert_eq!(
            leftover_volumes(prefix, &names, &models, &components),
            ["demo-1ab2c3_ck_auto_arima_chapkit_data"]
        );

        // And with both models enabled there is nothing left over: the
        // database and chap-core's own volumes are the base stack's.
        let both = vec![
            "chapkit_ewars_model".to_string(),
            "auto_arima_chapkit".to_string(),
        ];
        assert!(leftover_volumes(prefix, &names, &both, &components).is_empty());
    }

    #[test]
    fn the_stack_line_says_when_the_container_itself_is_unhealthy() {
        let mut report = status_report(
            ApiHealth::Down {
                error: "connection refused".to_string(),
            },
            &[],
            &[],
        );
        // Without a container to blame it is the port that is not answering.
        let (status, detail, fix) = stack_verdict(&report);
        assert_eq!(status, Status::Fail);
        assert!(detail.contains("is down: connection refused"), "{detail}");
        assert!(fix.unwrap().contains("chaps logs chap"));

        // With one, the check says which half is wrong and what its log said.
        report.unhealthy = vec![crate::diagnose::Unhealthy::of(
            API_SERVICE,
            "FATAL:  password authentication failed for user \"chap\"\n",
        )];
        let (status, detail, fix) = stack_verdict(&report);
        assert_eq!(status, Status::Fail);
        assert!(
            detail.starts_with("the chap container is up and unhealthy:"),
            "{detail}"
        );
        assert!(
            detail.contains("password authentication failed"),
            "{detail}"
        );
        assert_eq!(detail.lines().count(), 1, "one line per check: {detail}");
        assert!(fix.unwrap().contains("chaps down --volumes"));
    }

    /// The claims a two-model deployment would make.
    fn claim(service: &str, port: u16) -> PortClaim {
        PortClaim {
            service: service.to_string(),
            port,
        }
    }

    #[test]
    fn a_port_is_only_a_conflict_when_someone_else_holds_it() {
        let nothing = BTreeSet::new();
        let free = |_: u16| false;
        let taken = |_: u16| true;

        let api = claim(API_SERVICE, 8000);
        let check = port_check(&api, &nothing, &free, None, None);
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.id, "api-port");
        assert_eq!(check.name, "api port");
        assert_eq!(check.detail, "8000 is free");

        let check = port_check(&api, &nothing, &taken, Some(8001), None);
        assert_eq!(check.status, Status::Fail);
        assert_eq!(check.detail, "8000 is in use by something else");
        assert_eq!(
            check.fix.unwrap(),
            ports::busy_line(&api, Some(8001)),
            "the fix is the sentence `chaps up` would have failed with"
        );

        // A port this project's own container publishes is ours, exactly as
        // the `up` preflight treats it.
        let running: BTreeSet<String> = [API_SERVICE.to_string()].into_iter().collect();
        let check = port_check(&api, &running, &taken, None, None);
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("this project's own container"));

        let model = claim("chapkit-ewars-model", 5001);
        let check = port_check(&model, &nothing, &taken, None, None);
        assert_eq!(check.id, "port-chapkit-ewars-model");
        assert_eq!(check.name, "port chapkit-ewars-model");
        assert!(check.fix.unwrap().contains("chaps models unexpose"));
    }

    /// `.env` moving the API port is the operator's doing, so the line names
    /// the file rather than looking like a number out of nowhere.
    #[test]
    fn the_api_port_line_names_the_file_that_moved_the_port() {
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project {
            dir: dir.path().to_path_buf(),
            state: crate::project::ProjectState::default(),
        };
        project.state.api_port = 8000;

        // Nothing in .env: the recorded port stands, and there is nothing to
        // explain.
        assert_eq!(api_port_note(&project), None);
        // The line compose reads agrees with the recorded one: still nothing.
        std::fs::write(dir.path().join(ENV_FILE), "CHAP_API_PORT=8000\n").unwrap();
        assert_eq!(api_port_note(&project), None);

        std::fs::write(dir.path().join(ENV_FILE), "CHAP_API_PORT=18000\n").unwrap();
        let note = api_port_note(&project).expect("an override is worth a word");
        assert_eq!(
            note,
            "from .env, over the 8000 recorded in .chaps/project.yaml"
        );

        let claim = claim(API_SERVICE, project.effective_api_port());
        let check = port_check(&claim, &BTreeSet::new(), &|_| false, None, Some(&note));
        assert_eq!(
            check.detail,
            "18000 is free (from .env, over the 8000 recorded in .chaps/project.yaml)"
        );
    }

    #[test]
    fn the_pin_check_only_speaks_up_for_a_release_that_moved() {
        let (status, detail, fix) = pin_verdict("latest", ReleaseList::Newest("v2.3.1"));
        assert_eq!(status, Status::Ok, "a moving tag is not behind anything");
        assert!(detail.contains("moving tag"), "{detail}");
        assert_eq!(fix, None);

        let (status, detail, fix) = pin_verdict("v2.3.0", ReleaseList::Newest("v2.3.1"));
        assert_eq!(status, Status::Warn);
        assert_eq!(detail, "v2.3.0 pinned, v2.3.1 released");
        assert!(fix.unwrap().contains("chaps update --dry-run"));

        assert_eq!(
            pin_verdict("v2.3.1", ReleaseList::Newest("v2.3.1")).0,
            Status::Ok
        );
        assert_eq!(
            pin_verdict("v2.4.0", ReleaseList::Newest("v2.3.1")).0,
            Status::Ok
        );

        // Without the release list there is nothing to compare against, and
        // the two silent cases each say which one they are.
        let (status, detail, _) = pin_verdict("v2.3.0", ReleaseList::Unreachable);
        assert_eq!(status, Status::Skip);
        assert!(detail.contains("v2.3.0"), "{detail}");
        assert!(detail.contains("not reachable"), "{detail}");

        let (status, detail, _) = pin_verdict("v2.3.0", ReleaseList::Offline);
        assert_eq!(status, Status::Skip);
        assert!(
            detail.contains("v2.3.0") && detail.contains("offline"),
            "{detail}"
        );

        // A moving tag needs no list at all, offline included.
        assert_eq!(pin_verdict("latest", ReleaseList::Offline).0, Status::Ok);
    }

    /// The reason the image lines give, in the order the reasons rank.
    #[test]
    fn offline_is_why_the_image_lines_were_skipped_even_without_docker() {
        let reachable = Probed {
            ghcr: Ok(401),
            marketplace: Ok(200),
            chap_core: Ok("v2.3.1".to_string()),
            chaps: Ok("v0.2.0".to_string()),
        };

        // `--offline` outranks a missing docker CLI: the flag is the reason
        // the user gave, and the line reads the same on either machine.
        for have_cli in [true, false] {
            let reason = image_skip_reason(None, have_cli).expect("offline skips the lookup");
            assert!(reason.contains("offline"), "{reason}");
        }

        // Online, a machine without docker has nothing to ask through.
        assert_eq!(
            image_skip_reason(Some(&reachable), false).as_deref(),
            Some("no docker CLI to ask")
        );

        // Online with docker: only an unreachable registry holds it back.
        assert_eq!(image_skip_reason(Some(&reachable), true), None);
        let unreachable = Probed {
            ghcr: Err("dns error".to_string()),
            ..reachable
        };
        let reason = image_skip_reason(Some(&unreachable), true).expect("ghcr is down");
        assert!(reason.contains("dns error"), "{reason}");
    }

    #[test]
    fn a_manifest_list_is_searched_for_linux_amd64() {
        let list = r#"{"manifests":[
            {"platform":{"architecture":"amd64","os":"linux"}},
            {"platform":{"architecture":"arm64","os":"linux"}}
        ]}"#;
        assert_eq!(manifest_has_amd64(list), Some(true));

        let arm_only = r#"{"manifests":[{"platform":{"architecture":"arm64","os":"linux"}}]}"#;
        assert_eq!(manifest_has_amd64(arm_only), Some(false));

        // A single image manifest carries no list, so the platform is simply
        // not something this can answer.
        let single =
            r#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json"}"#;
        assert_eq!(manifest_has_amd64(single), None);
        assert_eq!(manifest_has_amd64("not json"), None);
    }

    #[test]
    fn an_image_that_is_not_there_names_the_model_to_re_resolve() {
        let check = image_verdict(
            "chapkit_ewars_model",
            "chapkit-ewars-model",
            "ghcr.io/chap-models/chapkit_ewars_model:sha-fa880a1",
            &done(false, "", "manifest unknown"),
        );
        assert_eq!(check.status, Status::Fail);
        assert_eq!(check.id, "image-chapkit-ewars-model");
        assert!(
            check.detail.contains("manifest unknown"),
            "{}",
            check.detail
        );
        assert_eq!(
            check.fix.unwrap(),
            "run `chaps models enable chapkit_ewars_model` to resolve the pin again"
        );

        let list = r#"{"manifests":[{"platform":{"architecture":"amd64","os":"linux"}}]}"#;
        let check = image_verdict("m", "svc", "ghcr.io/x:1", &done(true, list, ""));
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("linux/amd64"));

        let arm = r#"{"manifests":[{"platform":{"architecture":"arm64","os":"linux"}}]}"#;
        let check = image_verdict("m", "svc", "ghcr.io/x:1", &done(true, arm, ""));
        assert_eq!(check.status, Status::Warn);
        assert!(check.fix.unwrap().contains("chaps models enable m"));

        // An old CLI that hides `docker manifest` cannot answer, which is not
        // the same as the tag being gone.
        let check = image_verdict(
            "m",
            "svc",
            "ghcr.io/x:1",
            &done(
                false,
                "",
                "docker manifest is only supported on a Docker cli with \
                  experimental cli features enabled",
            ),
        );
        assert_eq!(check.status, Status::Skip);
        assert_eq!(
            image_verdict("m", "svc", "r", &Outcome::TimedOut).status,
            Status::Skip
        );
        assert_eq!(
            image_verdict("m", "svc", "r", &Outcome::Missing).status,
            Status::Skip
        );
    }

    /// A `StatusReport` with the fields the stack check reads.
    fn status_report(api: ApiHealth, expected: &[&str], missing: &[&str]) -> StatusReport {
        StatusReport {
            project: Some("mychap-1ab2c3".to_string()),
            api_url: "http://localhost:8000".to_string(),
            api_port: 8000,
            api_port_source: ApiPortSource::Project,
            api,
            version: ApiVersion {
                value: "v2.3.1".to_string(),
                pinned: false,
            },
            registered: Vec::new(),
            expected: expected.iter().map(|s| s.to_string()).collect(),
            missing: missing.iter().map(|s| s.to_string()).collect(),
            reach: BTreeMap::new(),
            models: Vec::<ModelStatus>::new(),
            unmanaged: Vec::new(),
            auth: false,
            components: Vec::new(),
            unhealthy: Vec::new(),
        }
    }

    #[test]
    fn the_stack_check_follows_what_status_found() {
        let up = || ApiHealth::Up {
            status: "ok".to_string(),
            message: String::new(),
        };

        let (status, detail, fix) = stack_verdict(&status_report(up(), &["a", "b"], &[]));
        assert_eq!(status, Status::Ok);
        assert_eq!(
            detail,
            "chap-core up at http://localhost:8000, 2 of 2 models registered"
        );
        // Green, and still with something to do: registration is a heartbeat.
        assert_eq!(fix.as_deref(), Some(crate::status::TEST_HINT));

        // A deployment with no models has nothing to test.
        let (status, _, fix) = stack_verdict(&status_report(up(), &[], &[]));
        assert_eq!(status, Status::Ok);
        assert_eq!(fix, None);

        let (status, detail, fix) = stack_verdict(&status_report(up(), &["a", "b"], &["b"]));
        assert_eq!(status, Status::Warn, "a missing model is not a dead stack");
        assert!(detail.ends_with("not registered: b"), "{detail}");
        assert!(fix.unwrap().contains("chaps status"));

        let down = ApiHealth::Down {
            error: "connection refused".to_string(),
        };
        let (status, detail, fix) = stack_verdict(&status_report(down, &["a"], &["a"]));
        assert_eq!(status, Status::Fail);
        assert!(detail.contains("connection refused"), "{detail}");
        assert!(fix.unwrap().contains("chaps logs chap"));
    }

    #[test]
    fn a_bounded_run_reports_a_binary_that_is_not_there() {
        let outcome = run_bounded("chaps-no-such-binary-exists", &["--help"], DOCKER_TIMEOUT);
        assert_eq!(outcome, Outcome::Missing);
        assert!(!outcome.succeeded());
        assert_eq!(outcome.stderr(), "the binary is not on PATH");
    }

    #[test]
    fn a_bounded_run_captures_what_the_command_said() {
        // `cargo` is what is running this test, so it is on PATH by
        // construction, and `--version` is the same two words everywhere.
        let outcome = run_bounded("cargo", &["--version"], DOCKER_TIMEOUT);
        let Outcome::Done { ok, stdout, .. } = &outcome else {
            panic!("expected a finished command, got {outcome:?}");
        };
        assert!(ok);
        assert!(stdout.starts_with("cargo "), "{stdout}");
        assert!(outcome.succeeded());
    }

    #[test]
    fn long_output_is_cut_to_one_readable_line() {
        assert_eq!(first_line("\n\n  hello  \nworld\n"), "hello");
        assert_eq!(first_line(""), "");
        let long = "x".repeat(400);
        let cut = first_line(&long);
        assert_eq!(cut.chars().count(), 120);
        assert!(cut.ends_with("..."));
    }
}
