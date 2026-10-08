//! `varde self update|version` and the once-a-day update notice.
//!
//! The mechanics live in [`crate::selfupdate`]; this module is the command
//! surface on top of them: what is printed, what `--json` carries, and what a
//! refusal says.

use crate::cli::{SelfUpdateArgs, SelfVersionArgs};
use crate::commands::Ctx;
use crate::error::Result;
use crate::output;
use crate::selfupdate::{self, Channel, GIT_REVISION, TARGET, VERSION};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long `self update` waits on the release feed and the download.
const TIMEOUT: Duration = Duration::from_secs(60);

/// `varde self version`, and the shape of its `--json`.
#[derive(Debug, serde::Serialize)]
struct VersionReport {
    version: &'static str,
    /// Short commit of the checkout this was built from, absent when there
    /// was none to record.
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<&'static str>,
    /// `stable` or `dev`. Always present, in the text output as well as in
    /// `--json`: a field that only appears on a dev build would make its
    /// absence the thing to notice, and the whole point of this command is to
    /// say what a build is without anyone having to know that.
    channel: &'static str,
    target: &'static str,
    /// The running executable, as the OS reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
    /// `release archive`, `cargo install` or `cargo build`, guessed from the path.
    install_method: &'static str,
}

/// `varde self update`, and the shape of its `--json`.
#[derive(Debug, serde::Serialize)]
struct UpdateReport {
    /// The version before the run.
    current: String,
    /// The channel this build follows, which is what decides whether `latest`
    /// is the newest tag or the newest rolling build.
    channel: &'static str,
    /// The release that was looked up.
    latest: String,
    /// Whether `latest` is newer than `current`.
    update_available: bool,
    /// Whether this run replaced the binary.
    updated: bool,
    /// The asset the target wants, once a release is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    asset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
}

/// `varde self version`: everything that identifies one build.
pub fn version(ctx: &Ctx, _args: &SelfVersionArgs) -> Result<()> {
    let path = std::env::current_exe().ok();
    let report = VersionReport {
        version: VERSION,
        revision: Some(GIT_REVISION).filter(|rev| !rev.is_empty()),
        channel: selfupdate::channel().as_str(),
        target: TARGET,
        path: path.clone(),
        install_method: path
            .as_deref()
            .map(selfupdate::install_method)
            .unwrap_or("release archive"),
    };
    ctx.out.emit(&report, || {
        let mut fields = vec![
            ("version", ctx.out.value(&format!("v{}", report.version))),
            ("channel", report.channel.to_string()),
            ("target", report.target.to_string()),
        ];
        if let Some(revision) = report.revision {
            fields.insert(1, ("revision", revision.to_string()));
        }
        fields.push((
            "path",
            ctx.out.dim(
                &report
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
        ));
        fields.push(("installed by", report.install_method.to_string()));
        output::fields_with(2, &fields, &|label| ctx.out.key(label))
    })
}

/// `varde self update`: look up a release, then install it unless `--check`.
pub fn update(ctx: &Ctx, args: &SelfUpdateArgs) -> Result<()> {
    if ctx.registry.offline {
        return Err(anyhow::anyhow!(
            "--offline and `varde self update` ask for opposite things; \
             an update is a download"
        ));
    }

    // A dev build follows the rolling `dev` pre-release rather than
    // `releases/latest`, which GitHub documents as excluding pre-releases.
    // `--version TAG` always wins, `dev` included: that is how a stable build
    // crosses over, and how a dev build goes back to a numbered release.
    let channel = selfupdate::channel();
    let release = match (&args.version, channel.tag()) {
        (Some(tag), _) => selfupdate::release_by_tag(tag, TIMEOUT)?,
        (None, Some(tag)) => selfupdate::release_by_tag(tag, TIMEOUT)?,
        (None, None) => selfupdate::latest_release(TIMEOUT)?,
    };

    // Without --version the run is about moving forward, so an equal or older
    // tag is nothing to do. With --version the tag was asked for by name and
    // going back is allowed; only landing where we already are is not.
    //
    // The `dev` tag never moves, so neither of those questions can be asked of
    // it as a version: what separates two rolling builds is the commit, which
    // the release notes record and the running build knows as GIT_REVISION.
    let wanted = args.version.is_some();
    let rolling = release.tag == selfupdate::DEV_TAG;
    let newer = if rolling {
        selfupdate::dev_update_available(&release, GIT_REVISION)
    } else {
        selfupdate::is_newer_than_current(&release.tag)
    };
    let same = already_running(channel, &release.tag, VERSION, newer);

    let asset = selfupdate::pick_asset(&release, TARGET);
    // The file itself, not the `vg` link to it: an update through the short
    // form must replace `varde`, which the link then still points at.
    let path = std::env::current_exe()
        .ok()
        .map(|exe| std::fs::canonicalize(&exe).unwrap_or(exe));

    if (!wanted && !newer) || same {
        let report = UpdateReport {
            current: format!("v{VERSION}"),
            channel: channel.as_str(),
            latest: release.tag.clone(),
            update_available: newer,
            updated: false,
            asset: asset.ok(),
            path: path.clone(),
        };
        return ctx.out.report(&report, |lines| {
            lines.info(format!("varde {} is up to date", describe_build()));
        });
    }

    // The way out depends on how the release was picked: a tag asked for by
    // name has a newer one, and the newest release has nowhere else to go.
    let asset = asset.map_err(|err| match &args.version {
        Some(_) => anyhow::anyhow!(
            "{err}; pick a newer tag with --version, or run `varde self update` for the newest"
        ),
        None => anyhow::anyhow!(
            "{err}; download a build from https://github.com/{}/releases",
            selfupdate::REPO
        ),
    })?;

    if args.check {
        let report = UpdateReport {
            current: format!("v{VERSION}"),
            channel: channel.as_str(),
            latest: release.tag.clone(),
            update_available: true,
            updated: false,
            asset: Some(asset.clone()),
            path: path.clone(),
        };
        return ctx.out.report(&report, |lines| {
            lines
                .info(format!(
                    "varde {} is available (you have {})",
                    describe_release(&release),
                    describe_build()
                ))
                .hint(format!("the download is {asset}"))
                .hint(format!("`{}` installs it", install_command(args)));
        });
    }

    let path = path.ok_or_else(|| {
        anyhow::anyhow!("this platform does not say where the running binary is; install by hand")
    })?;
    let path = std::fs::canonicalize(&path).unwrap_or(path);

    if !selfupdate::is_replaceable(&path) {
        return Err(anyhow::anyhow!(
            "{} is not writable, so varde cannot replace itself there; \
             re-run with sudo, or install into a directory you own with \
             `curl -fsSL https://raw.githubusercontent.com/{}/main/install.sh | \
             VARDE_INSTALL_DIR=$HOME/.local/bin sh`",
            path.display(),
            selfupdate::REPO
        ));
    }

    if !confirm(ctx, args, &release.tag, &path)? {
        let report = UpdateReport {
            current: format!("v{VERSION}"),
            channel: channel.as_str(),
            latest: release.tag.clone(),
            update_available: true,
            updated: false,
            asset: Some(asset),
            path: Some(path),
        };
        return ctx.out.report(&report, |lines| {
            lines.info("cancelled; nothing was changed");
        });
    }

    install(ctx, &release.tag, &asset, &path)?;

    let report = UpdateReport {
        current: format!("v{VERSION}"),
        channel: channel.as_str(),
        latest: release.tag.clone(),
        update_available: true,
        updated: true,
        asset: Some(asset),
        path: Some(path.clone()),
    };
    ctx.out.report(&report, |lines| {
        lines
            .info(format!(
                "updated varde: {} -> {}",
                describe_build(),
                describe_release(&release)
            ))
            .hint(format!("replaced {}", path.display()));
    })
}

/// The command that installs what `--check` found: the same `--version` it
/// checked, because a plain `varde self update` follows this build's own
/// channel and can install something else.
fn install_command(args: &SelfUpdateArgs) -> String {
    match &args.version {
        Some(tag) => format!("varde self update --version {tag}"),
        None => "varde self update".to_string(),
    }
}

/// Whether the release that was looked up is the build already running, and
/// there is therefore nothing to install.
///
/// Crossing channels never counts as "already up to date", in either
/// direction: a dev build and the stable release that carries the same number
/// are different binaries, built from different commits, so `varde self update
/// --version v1.2.3` from a `v1.2.3` dev build has to install, and so does
/// `--version dev` from a stable build. Only a build that is already on the
/// exact thing the release would put there is up to date - which on the
/// rolling channel is a question about the commit, since the `dev` tag is the
/// same string every time, and `dev_newer` is that answer.
fn already_running(
    channel: Channel,
    release_tag: &str,
    running_version: &str,
    dev_newer: bool,
) -> bool {
    if release_tag == selfupdate::DEV_TAG {
        return channel == Channel::Dev && !dev_newer;
    }
    channel == Channel::Stable && release_tag.trim_start_matches('v') == running_version
}

/// How this build names itself in a sentence: the version, and for a rolling
/// build the channel and the commit, which is the only thing that tells two
/// of them apart.
fn describe_build() -> String {
    match selfupdate::channel() {
        Channel::Stable => format!("v{VERSION}"),
        Channel::Dev if GIT_REVISION.is_empty() => format!("v{VERSION} dev"),
        Channel::Dev => format!("v{VERSION} dev {GIT_REVISION}"),
    }
}

/// The same for a release that was looked up: the tag for a numbered release,
/// and for the rolling one the commit and the day it was built, because its
/// tag is the same string every time.
fn describe_release(release: &selfupdate::Release) -> String {
    if release.tag != selfupdate::DEV_TAG {
        return release.tag.clone();
    }
    let commit = selfupdate::release_commit(release);
    match (commit, release.published_day()) {
        (Some(commit), "") => format!("dev {}", &commit[..7]),
        (Some(commit), day) => format!("dev {} ({day})", &commit[..7]),
        (None, "") => "dev".to_string(),
        (None, day) => format!("dev ({day})"),
    }
}

/// Download, verify, unpack and swap. Everything before the swap happens in a
/// temporary directory under the cache directory, so a failure leaves the
/// installed binary untouched.
fn install(ctx: &Ctx, tag: &str, asset: &str, path: &Path) -> Result<()> {
    let base = selfupdate::download_base(tag);
    let work = ctx.registry.cache_dir.join("self-update").join(tag);
    // A previous failed run may have left files here.
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", work.display()))?;

    ctx.out.verbose(&format!("downloading {base}/{asset}"));
    let archive = selfupdate::download(&format!("{base}/{asset}"), TIMEOUT)?;
    let sums = selfupdate::download_text(&format!("{base}/{}", selfupdate::SUMS_FILE), TIMEOUT)?;
    selfupdate::verify(&sums, asset, &archive)?;
    ctx.out
        .verbose(&format!("{asset} matches {}", selfupdate::SUMS_FILE));

    let archive_path = work.join(asset);
    std::fs::write(&archive_path, &archive)
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", archive_path.display()))?;
    selfupdate::extract(&archive_path, &work)?;

    let binary = selfupdate::find_binary(&work, TARGET).ok_or_else(|| {
        anyhow::anyhow!(
            "{asset} does not contain {}; the release is not one this varde can install",
            selfupdate::binary_name(TARGET)
        )
    })?;
    let contents =
        std::fs::read(&binary).map_err(|e| anyhow::anyhow!("reading {}: {e}", binary.display()))?;

    selfupdate::replace_executable(path, &contents)?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(())
}

/// Ask before replacing the binary, unless there is nobody to ask.
///
/// `--yes`, `--json` and a non-terminal stdin all mean go ahead: a prompt
/// nobody can answer is a hang, not a safeguard.
fn confirm(ctx: &Ctx, args: &SelfUpdateArgs, tag: &str, path: &Path) -> Result<bool> {
    use std::io::{IsTerminal, Write};

    if args.yes || ctx.out.json || !std::io::stdin().is_terminal() || !ctx.out.tty {
        return Ok(true);
    }
    print!(
        "replace {} with varde {tag}? [y/N] ",
        ctx.out.value(&path.display().to_string())
    );
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"))
}

/// The once-a-day "a newer varde exists" line.
///
/// Called after a command succeeded. It is a courtesy, so every failure is
/// swallowed: a release feed that is down, a rate limit, a read-only cache
/// directory or a clock that moved must never change what a command did or
/// what it exited with. `-v` says what happened.
pub fn notify(ctx: &Ctx) {
    if !should_notify(ctx, selfupdate::checks_disabled()) {
        return;
    }
    let channel = selfupdate::channel();
    let cache_dir = ctx.registry.cache_dir.clone();
    let state = selfupdate::read_check(&cache_dir);
    let now = selfupdate::now_unix();

    let known = if selfupdate::is_stale(state.as_ref(), now, selfupdate::CHECK_INTERVAL) {
        // A dev build watches the rolling release; a stable one watches
        // `releases/latest`, which GitHub documents as skipping pre-releases,
        // so the two never see each other's builds.
        let looked_up = match channel.tag() {
            Some(tag) => selfupdate::release_by_tag(tag, selfupdate::CHECK_TIMEOUT),
            None => selfupdate::latest_release(selfupdate::CHECK_TIMEOUT),
        };
        match looked_up {
            Ok(release) => {
                let found = selfupdate::CheckState {
                    checked_at_unix: now,
                    // Belt and braces on top of that documented behaviour: a
                    // pre-release that somehow arrived here is recorded as
                    // nothing rather than as something to offer.
                    latest: if channel == Channel::Stable && release.prerelease {
                        String::new()
                    } else {
                        release.tag.clone()
                    },
                    // The tag alone says nothing on the dev channel, where it
                    // is the same string every time.
                    commit: selfupdate::release_commit(&release).unwrap_or_default(),
                };
                selfupdate::write_check(&cache_dir, &found);
                found
            }
            Err(e) => {
                ctx.out.verbose(&format!("update check failed: {e}"));
                // Record the attempt so a machine with no network does not
                // try again on every single command, keeping whatever the
                // last successful check found.
                selfupdate::write_check(
                    &cache_dir,
                    &selfupdate::CheckState {
                        checked_at_unix: now,
                        ..state.unwrap_or_default()
                    },
                );
                return;
            }
        }
    } else {
        state.unwrap_or_default()
    };

    let line = match channel {
        Channel::Stable => selfupdate::notice_line(&known.latest, VERSION),
        Channel::Dev => selfupdate::dev_notice_line(&known.commit, GIT_REVISION),
    };
    if let Some(line) = line {
        output::notice(&line);
    }
}

/// Whether this invocation is one that may print the notice.
///
/// Not under `--json` (a second stream a parser did not ask for), not when
/// stdout is not a terminal (a pipe, a CI log, a test), not under `--offline`
/// or `VARDE_NO_UPDATE_CHECK=1`, and not for `varde self ...`, which is the
/// command the notice would be telling you to run.
fn should_notify(ctx: &Ctx, disabled: bool) -> bool {
    ctx.out.tty && !ctx.out.json && !ctx.registry.offline && !disabled
}

#[cfg(test)]
mod tests;
