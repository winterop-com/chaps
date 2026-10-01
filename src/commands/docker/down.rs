//! `chaps down`, and `--volumes` above all: the one wrapper that destroys
//! data, so it names what will go, asks first and reports what went.

use super::note;
use crate::cli::DownArgs;
use crate::commands::Ctx;
use crate::components::DHIS2_CONNECT_FORGOTTEN_WITH_VOLUME;
use crate::docker;
use crate::error::Result;
use crate::output::Out;
use crate::project::Project;
use std::io::{BufRead, IsTerminal, Write};

/// What `down --volumes` says when there is nobody to confirm to.
///
/// A prompt nobody can answer is a hang, and going ahead unasked would
/// destroy the data of whatever deployment the script happened to be in, so
/// the flag that cannot be undone is the one place `chaps` refuses instead.
const NO_TERMINAL: &str = "this destroys this deployment's data and there is no terminal to \
     confirm at; pass --yes";

/// The same under `--json`, which is a run nobody is watching either.
const NO_JSON_ANSWER: &str =
    "--json cannot ask before destroying this deployment's data; pass --yes";

/// What `down --volumes` asks before it runs.
const REMOVE_THEM: &str = "remove this deployment's volumes? [y/N] ";

/// The `-v`-shaped token an operator meant for compose's `--volumes`, if
/// `down` was given one.
///
/// Two ways in, and neither can be passed on. `chaps down -v` is taken by
/// clap as the global `--verbose` flag, so compose never sees it: the volumes
/// stay while the person who typed it believes they went, which is the whole
/// reason `--volumes` exists. Nothing in the parsed command line can tell
/// that apart from a deliberately verbose stop, so the raw arguments answer
/// it, and only the ones after `down` count - `chaps -v down` is a verbose
/// stop and stays one. Past a `--` the token reaches `EXTRA` instead, where
/// passing it through would destroy the data of an operator who only asked
/// for a louder `down`.
///
/// `chaps docker run -- down -v` is not this: it is the passthrough asking
/// for compose's own command line, where `-v` means what compose says it
/// means.
pub fn misused_volumes_flag<'a>(argv: &'a [String], extra: &'a [String]) -> Option<&'a str> {
    let typed = argv
        .iter()
        .position(|arg| arg == "down")
        .map_or(&[][..], |at| &argv[at + 1..]);
    typed
        .iter()
        .map(String::as_str)
        .find(|arg| *arg == "-v")
        .or_else(|| {
            extra
                .iter()
                .map(String::as_str)
                .find(|arg| matches!(*arg, "-v" | "--volumes" | "--volume"))
        })
}

/// What that refusal says: which flag it was, which flag does the job, and
/// how to ask for the verbose `down` the other reading would have given.
pub fn volumes_flag_message(token: &str) -> String {
    format!(
        "`{token}` after `down` is not passed on to compose (`-v` there is chaps's own \
         --verbose flag); `chaps down --volumes` removes this deployment's volumes and the \
         data in them, and `chaps -v down` is the verbose stop"
    )
}

/// Name the volumes `down --volumes` is about to destroy, and get a yes.
///
/// Returns the volumes docker holds under this deployment's prefix, which is
/// also what the closing line is measured against: compose removes the
/// volumes its files declare, so a leftover under the same prefix may well
/// outlive the run, and only docker can say which ones actually went.
///
/// `--yes` is how a script says it meant it. A run that cannot be asked - no
/// terminal on stdin, or `--json`, which nobody is watching - refuses rather
/// than assume, because there is no undoing this one.
pub(super) fn confirm_volumes(
    ctx: &Ctx,
    project: &Project,
    args: &DownArgs,
) -> Result<Vec<String>> {
    let prefix = project.volume_prefix();
    let volumes = match prefix.as_deref() {
        Some(prefix) => docker::volume_names_with_prefix(prefix),
        None => Vec::new(),
    };
    // How much the one volume that is usually large holds, so the prompt says
    // what is at stake and not only how many volumes there are. Only asked
    // for when docker has already said the volume is there, and bounded like
    // every other size: a `du` that will not finish costs the size and not
    // the prompt.
    // With `--yes` there is no question for the line to inform, and the
    // closing line names every volume that went.
    if args.yes {
        return Ok(volumes);
    }
    let ocs_data = prefix
        .as_deref()
        .and_then(|prefix| ocs_data_at_stake(prefix, &volumes));
    note(
        ctx,
        &volumes_at_stake(
            &ctx.out,
            &volumes,
            prefix.as_deref(),
            ocs_data
                .as_ref()
                .map(|(name, bytes)| (name.as_str(), *bytes)),
        ),
    );
    if ctx.out.json {
        return Err(anyhow::anyhow!(NO_JSON_ANSWER));
    }
    if !std::io::stdin().is_terminal() {
        return Err(anyhow::anyhow!(NO_TERMINAL));
    }
    eprint!("\n{REMOVE_THEM}");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| anyhow::anyhow!("reading the answer: {e}"))?;
    if matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
        return Ok(volumes);
    }
    Err(anyhow::anyhow!("cancelled; nothing was stopped"))
}

/// The line that names what is at stake, before `down --volumes` is let near
/// it.
///
/// Every volume under the deployment's prefix, because that is the set at
/// risk as docker sees it - and a statement of what docker holds rather than
/// a promise about what will go, since compose removes the volumes its files
/// still declare and a leftover under the same prefix may well outlive the
/// run. What did go is the closing line's to report.
///
/// An empty list is a line of its own rather than a silence: a deployment
/// that was never started has nothing to lose, and a docker that could not be
/// asked answers the same way, which is worth seeing before saying yes.
pub fn volumes_at_stake(
    out: &Out,
    volumes: &[String],
    prefix: Option<&str>,
    ocs_data: Option<(&str, u64)>,
) -> String {
    let under = match prefix {
        Some(prefix) => format!("under {prefix}*"),
        None => "under this deployment's name".to_string(),
    };
    if volumes.is_empty() {
        return out.dim(&format!("docker holds no volumes {under}"));
    }
    let mut line = format!(
        "{} {}",
        out.warn(&format!(
            "docker holds {} {under}, with their data:",
            volume_count(volumes.len())
        )),
        out.value(&volumes.join(", "))
    );
    // The names say what would go; the size says how much of it matters.
    if let Some((name, bytes)) = ocs_data {
        line.push_str(&format!(
            " {}",
            out.dim(&format!(
                "({name} holds {})",
                crate::backup::human_size(bytes)
            ))
        ));
    }
    line
}

/// This deployment's OCS data volume and what it holds, when docker holds one.
///
/// The size is the OCS one and no other because it is the one that grows: a
/// downloaded dataset store is gigabytes where the database and the model
/// volumes are megabytes, and it is the number that decides whether a `down
/// --volumes` is a keystroke or a mistake. `None` when there is no such
/// volume, or when the `du` could not be had.
pub(super) fn ocs_data_at_stake(prefix: &str, volumes: &[String]) -> Option<(String, u64)> {
    let name = format!("{prefix}{}", crate::compose::render::OCS_VOLUME);
    if !volumes.contains(&name) {
        return None;
    }
    let bytes = docker::volume_size_bytes(&name)?;
    Some((name, bytes))
}

/// `1 volume` or `N volumes`.
fn volume_count(count: usize) -> String {
    format!("{count} volume{}", if count == 1 { "" } else { "s" })
}

/// The line a `down` ends on: what brings the deployment back, and after a
/// `--volumes` that there is nothing left in the folder worth keeping.
pub fn down_next(volumes_removed: bool) -> &'static str {
    match volumes_removed {
        false => "run `chaps up` to start it again, with its data",
        true => {
            "run `chaps up` to start it again with empty data, or delete this folder: its containers \
             and volumes are gone"
        }
    }
}

/// Forget a recorded `chaps dhis2 connect` when this run has just removed the
/// database it was true of, and give back the line that says so.
///
/// `chaps down --volumes` is the one wrapper that destroys data, and `dhis2_db`
/// is one of the volumes it takes. The record left behind would then be a
/// record of a route in a database that no longer exists, suppressing the hint
/// on the next `chaps up` - which restores the seed dump, and that dump ships a
/// `chap` route pointing at a CHAP this deployment has nothing to do with.
///
/// Measured against what docker no longer holds rather than against the flag on
/// the command line: compose removes the volumes its own files declare, so a
/// `down --volumes` on a deployment whose `compose.dhis2.yml` has already gone
/// leaves `dhis2_db` exactly where it was. `None` when there was no record,
/// when this deployment can name no volumes, or when that volume survived - in
/// each case nothing was changed and there is nothing to say.
pub(super) fn forget_dhis2_connect(project: &mut Project, removed: &[String]) -> Option<String> {
    project.state.components.dhis2.connected_at.as_ref()?;
    let volume = project.prefixed_volume(crate::compose::render::DHIS2_DB_VOLUME)?;
    if !removed.contains(&volume) {
        return None;
    }
    project.state.components.dhis2.connected_at = None;
    // A state file that could not be written is worth a line of its own: the
    // record is still there, and the next `chaps up` will still be quiet about
    // connecting. The `down` itself succeeded and is not failed for it.
    if let Err(why) = project.save() {
        crate::output::warn(&format!(
            "the record of `chaps dhis2 connect` could not be cleared from \
             `.chaps/components.yaml`: {why}; run `chaps dhis2 connect` after the next `chaps up`"
        ));
        return None;
    }
    Some(
        match project.state.components.dhis2_seed_source() {
            Some(_) => DHIS2_CONNECT_FORGOTTEN_WITH_VOLUME,
            None => crate::components::DHIS2_CONNECT_FORGOTTEN_UNSEEDED,
        }
        .to_string(),
    )
}

/// What `down` did about the volumes, for the second half of its line.
#[derive(Debug, Clone, Copy)]
pub enum DownVolumes<'a> {
    /// A plain `down`: the data is all still there.
    Kept,
    /// `down --volumes`: the volumes docker no longer holds, which is what
    /// the run actually removed rather than what it set out to.
    Removed(&'a [String]),
}

/// The volumes of `before` that docker no longer holds.
///
/// Compose removes the volumes its own files declare, so a leftover from a
/// disabled model under the same prefix survives a `down --volumes`; asking
/// docker again is the only way to say which ones went.
pub(super) fn removed_volumes(project: &Project, before: &[String]) -> Vec<String> {
    let left = match project.volume_prefix() {
        Some(prefix) => docker::volume_names_with_prefix(&prefix),
        None => Vec::new(),
    };
    before
        .iter()
        .filter(|name| !left.contains(name))
        .cloned()
        .collect()
}

/// What `down` stopped, and what became of the volumes.
///
/// The volumes are the point of the second half: `down` is the command people
/// reach for to "reset" a deployment, and it keeps the database. The compose
/// project name goes with them, because it is the prefix those volumes carry
/// and therefore what `docker volume ls` has to be asked about. A
/// `--volumes` run names what it removed instead, by name: the deployment
/// whose data is gone is not the place for a count alone.
pub fn down_summary(
    out: &Out,
    stopped: &[String],
    volumes: DownVolumes,
    project_name: Option<&str>,
) -> String {
    let head = if stopped.is_empty() {
        out.dim("nothing was running")
    } else {
        format!(
            "{} {}",
            out.warn("stopped:"),
            out.dim(&format!(
                "{} ({} container{})",
                stopped.join(", "),
                stopped.len(),
                if stopped.len() == 1 { "" } else { "s" }
            ))
        )
    };
    match volumes {
        // Nothing ran and nothing was asked about the volumes: one clause
        // says everything there is to say.
        DownVolumes::Kept if stopped.is_empty() => head,
        DownVolumes::Kept => {
            let kept = match project_name {
                Some(name) => {
                    format!("volumes kept: {name}_* (`chaps down --volumes` removes them)")
                }
                None => "volumes kept (`chaps down --volumes` removes them)".to_string(),
            };
            format!("{head}; {}", out.backticks(&kept))
        }
        DownVolumes::Removed([]) => format!("{head}; {}", out.dim("no volumes were removed")),
        DownVolumes::Removed(names) => format!(
            "{head}; {} {}",
            out.warn(&format!("removed {}", volume_count(names.len()))),
            out.value(&format!("({})", names.join(", ")))
        ),
    }
}
