//! Checks of the machine: docker, compose, the platform, disk and memory.

use super::*;

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
            format!("Docker Engine {}", info_fields(stdout).engine),
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
        format!("{detail}; Chap images are linux/amd64 only, so they run under emulation"),
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
                "free up space: the Chap images total several GB, and below {} a pull stops \
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
                "keep at least {} free: the Chap images total several GB",
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

/// Whether there is memory for DHIS2.
///
/// `memory` is what the docker engine says it can give a container, which is
/// the only figure that answers the question on every platform: on macOS and
/// Windows the containers live in a Linux VM whose size the host's own free
/// memory says nothing about, and on Linux it is the host's memory. An engine
/// that would not say is a skip - the figure is not one to guess at, and a
/// wrong answer here sends an operator after a memory problem they do not have,
/// or leaves them without the one they do.
pub fn memory_verdict(memory: Option<u64>) -> (Status, String, Option<String>) {
    let Some(memory) = memory else {
        return (
            Status::Skip,
            "docker did not say how much memory it can give a container".to_string(),
            None,
        );
    };
    let detail = format!("{} available to docker", human_bytes(memory));
    if memory < DHIS2_MEMORY_FAIL {
        return (
            Status::Fail,
            detail,
            Some(format!(
                "give docker at least {} (Docker Desktop: Settings, Resources), or lower the heap \
                 with `{DHIS2_JAVA_ENV_VAR}` in `.env`: below {} the JVM is killed part-way \
                 through the analytics populate phase, and the run never finishes",
                human_bytes(DHIS2_MEMORY_WARN),
                human_bytes(DHIS2_MEMORY_FAIL),
            )),
        );
    }
    if memory < DHIS2_MEMORY_WARN {
        return (
            Status::Warn,
            detail,
            Some(format!(
                "keep at least {} for docker: DHIS2 alone wants {} for the analytics populate \
                 phase, and `{DHIS2_JAVA_ENV_VAR}` in `.env` is where its heap is sized",
                human_bytes(DHIS2_MEMORY_WARN),
                human_bytes(DHIS2_MEMORY_FAIL),
            )),
        );
    }
    (Status::Ok, detail, None)
}

/// The `memory` line, which has nothing to ask when there is no CLI to ask it
/// through.
pub fn memory_check(have_cli: bool, memory: Option<u64>) -> Check {
    if !have_cli {
        return Check::skip("memory", "memory", "no docker CLI to ask");
    }
    Check::from_verdict("memory", "memory", memory_verdict(memory))
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
pub(super) fn free_bytes(path: &Path) -> Option<u64> {
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

/// What the one `docker info` said, split out of what [`INFO_FORMAT`] asked
/// for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DockerInfo {
    /// The engine version, for the `docker-daemon` line.
    pub engine: String,
    /// Docker's own data root, for `disk`. `None` when the engine did not say.
    pub root_dir: Option<String>,
    /// How much memory the engine can give a container, in bytes. `None` when
    /// the engine did not say, or said nothing that reads as a number.
    pub memory: Option<u64>,
}

/// Split one `docker info` line into [`DockerInfo`].
///
/// Any field may be missing: an engine that does not know one prints it empty
/// rather than dropping it, and an older engine may print fewer fields than
/// were asked for. A memory of `0` is "would not say" rather than a machine
/// with no memory, which is not a thing.
pub fn info_fields(stdout: &str) -> DockerInfo {
    let mut fields = stdout.trim().split('\t');
    let engine = fields.next().unwrap_or_default().trim().to_string();
    let root_dir = fields
        .next()
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(str::to_string);
    let memory = fields
        .next()
        .map(str::trim)
        .and_then(|bytes| bytes.parse::<u64>().ok())
        .filter(|bytes| *bytes > 0);
    DockerInfo {
        engine,
        root_dir,
        memory,
    }
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

#[cfg(test)]
mod tests;
