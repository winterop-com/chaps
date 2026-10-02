//! `chaps cleanup`: remove what deployments that no longer exist left in
//! docker - their volumes and their network.
//!
//! Only what chaps can prove is left over goes. A deployment counts as gone
//! when chaps recorded it ([`crate::known`]) and its directory no longer holds
//! it; a deployment that is only down still has its directory, and its data
//! stays. Volumes chaps never recorded a deployment for are not touched, and
//! neither is anything a container still uses.

use crate::cli::CleanupArgs;
use crate::commands::Ctx;
use crate::docker;
use crate::error::Result;
use serde::Serialize;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

/// One deployment that is gone, and what it left.
#[derive(Debug, Clone, Serialize)]
pub struct Leftover {
    /// The compose project name it ran under.
    pub project: String,
    /// The directory it was in, which no longer holds it.
    pub dir: PathBuf,
    /// Its volumes no container uses.
    pub volumes: Vec<String>,
    /// Its network, `<project>_default`, when docker still has it.
    pub network: Option<String>,
}

/// A gone deployment left as it is, and why.
#[derive(Debug, Clone, Serialize)]
pub struct Kept {
    pub project: String,
    pub dir: PathBuf,
    pub reason: String,
}

/// What looking found.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Findings {
    pub leftovers: Vec<Leftover>,
    pub kept: Vec<Kept>,
    /// Gone deployments that left nothing, only a line in the record.
    pub forgotten: Vec<String>,
    /// How many recorded deployments are still in place.
    pub present: usize,
}

impl Findings {
    /// How many volumes the leftovers hold.
    pub fn volume_count(&self) -> usize {
        self.leftovers.iter().map(|l| l.volumes.len()).sum()
    }
}

/// What `chaps cleanup` did, for `--json`.
#[derive(Debug, Serialize)]
struct CleanupReport {
    dry_run: bool,
    #[serde(flatten)]
    findings: Findings,
    removed_volumes: Vec<String>,
    removed_networks: Vec<String>,
    refused: Vec<Refusal>,
}

#[derive(Debug, Serialize)]
struct Refusal {
    volume: String,
    why: String,
}

/// Look for what gone deployments left, without changing anything. `None`
/// when docker could not be asked, which is not the same as nothing found.
pub fn find() -> Option<Findings> {
    let mut findings = Findings::default();
    for (project, dir) in crate::known::load() {
        match crate::known::presence(&project, &dir) {
            crate::known::Presence::Present => {
                findings.present += 1;
                continue;
            }
            crate::known::Presence::Unsure(reason) => {
                findings.kept.push(Kept {
                    project,
                    dir,
                    reason,
                });
                continue;
            }
            crate::known::Presence::Gone => {}
        }
        if docker::project_has_containers(&project)? {
            findings.kept.push(Kept {
                reason: format!(
                    "containers of {project} still exist; `docker compose -p {project} down` \
                     removes them"
                ),
                project,
                dir,
            });
            continue;
        }
        let volumes: Vec<String> = docker::volume_names_of_project(&project)
            .into_iter()
            .filter(|v| docker::volume_in_use(v) == Some(false))
            .collect();
        let network =
            docker::default_network_exists(&project).then(|| format!("{project}_default"));
        if volumes.is_empty() && network.is_none() {
            findings.forgotten.push(project);
            continue;
        }
        findings.leftovers.push(Leftover {
            project,
            dir,
            volumes,
            network,
        });
    }
    Some(findings)
}

pub fn run(ctx: &Ctx, args: &CleanupArgs) -> Result<()> {
    let Some(findings) = find() else {
        return Err(anyhow::anyhow!(
            "docker is not answering, so nothing was checked; start Docker, then `chaps cleanup` \
             again"
        ));
    };
    let mut report = CleanupReport {
        dry_run: args.dry_run,
        findings,
        removed_volumes: Vec::new(),
        removed_networks: Vec::new(),
        refused: Vec::new(),
    };
    if report.findings.leftovers.is_empty() {
        if !args.dry_run {
            crate::known::forget(&report.findings.forgotten)?;
        }
        return ctx.out.emit_ok(&report, || summary(ctx, &report));
    }
    if !args.dry_run {
        if !args.yes {
            confirm(ctx, &report.findings)?;
        }
        remove(&mut report);
    }
    ctx.out.emit_ok(&report, || summary(ctx, &report))
}

/// Remove every leftover, and forget each deployment that left nothing
/// behind once it is done.
fn remove(report: &mut CleanupReport) {
    let mut done = report.findings.forgotten.clone();
    for leftover in &report.findings.leftovers {
        let mut all_gone = true;
        for volume in &leftover.volumes {
            match docker::remove_volume(volume) {
                docker::Removal::Removed => report.removed_volumes.push(volume.clone()),
                docker::Removal::NotFound => {}
                docker::Removal::Refused(why) => {
                    all_gone = false;
                    report.refused.push(Refusal {
                        volume: volume.clone(),
                        why,
                    });
                }
            }
        }
        if let Some(network) = &leftover.network {
            docker::remove_default_network(&leftover.project);
            report.removed_networks.push(network.clone());
        }
        if all_gone {
            done.push(leftover.project.clone());
        }
    }
    if let Err(err) = crate::known::forget(&done) {
        crate::output::warn(&format!("{err:#}"));
    }
}

/// Name what will go, and get a yes. Nobody to ask - `--json`, or no
/// terminal - is a refusal, as for `chaps down --volumes`: there is no
/// undoing this.
fn confirm(ctx: &Ctx, findings: &Findings) -> Result<()> {
    if ctx.out.json {
        return Err(anyhow::anyhow!(
            "--json cannot ask before deleting data; `chaps cleanup --dry-run` lists it, and \
             `chaps cleanup --yes` deletes it"
        ));
    }
    if !std::io::stdin().is_terminal() {
        return Err(anyhow::anyhow!(
            "this deletes data and there is no terminal to confirm at; `chaps cleanup --yes` \
             deletes it without asking"
        ));
    }
    eprint!("{}", listing(ctx, findings));
    eprint!(
        "\ndelete {} volume{} of {} removed deployment{}? [y/N] ",
        findings.volume_count(),
        plural(findings.volume_count()),
        findings.leftovers.len(),
        plural(findings.leftovers.len())
    );
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| anyhow::anyhow!("reading the answer: {e}"))?;
    match matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
        true => Ok(()),
        false => Err(anyhow::anyhow!("cancelled; nothing was removed")),
    }
}

/// Each gone deployment with what it left.
fn listing(ctx: &Ctx, findings: &Findings) -> String {
    let mut text = String::new();
    for leftover in &findings.leftovers {
        text.push_str(&format!(
            "{} (was {})\n",
            leftover.project,
            ctx.out.dim(&leftover.dir.display().to_string())
        ));
        for volume in &leftover.volumes {
            text.push_str(&format!("  volume {volume}\n"));
        }
        if let Some(network) = &leftover.network {
            text.push_str(&format!("  network {network}\n"));
        }
    }
    text
}

fn summary(ctx: &Ctx, report: &CleanupReport) -> String {
    let findings = &report.findings;
    let mut text = String::new();
    for kept in &findings.kept {
        text.push_str(&ctx.out.backticks(&format!(
            "kept {} (was {}): {}\n",
            kept.project,
            kept.dir.display(),
            kept.reason
        )));
    }
    if findings.leftovers.is_empty() {
        let forgot = match findings.forgotten.len() {
            0 => String::new(),
            n if report.dry_run => format!("; {n} removed deployment{} left nothing", plural(n)),
            n => format!(
                "; forgot {n} removed deployment{} that left nothing",
                plural(n)
            ),
        };
        text.push_str(&format!(
            "nothing to clean up: {} recorded deployment{} still in place{forgot}",
            findings.present,
            match findings.present {
                1 => " is",
                _ => "s are",
            },
        ));
        return text;
    }
    if report.dry_run {
        text.push_str(&listing(ctx, findings));
        text.push_str(&ctx.out.backticks(&format!(
            "{} volume{} of {} removed deployment{} to delete; `chaps cleanup` deletes them",
            findings.volume_count(),
            plural(findings.volume_count()),
            findings.leftovers.len(),
            plural(findings.leftovers.len())
        )));
        return text;
    }
    for volume in &report.removed_volumes {
        text.push_str(&format!("{} volume {volume}\n", ctx.out.warn("removed")));
    }
    for network in &report.removed_networks {
        text.push_str(&format!("{} network {network}\n", ctx.out.warn("removed")));
    }
    for refusal in &report.refused {
        text.push_str(&ctx.out.backticks(&format!(
            "kept volume {}: {}; `docker ps -a --filter volume={}` shows what holds it\n",
            refusal.volume, refusal.why, refusal.volume
        )));
    }
    text.push_str(&format!(
        "cleaned up {} removed deployment{}; {} recorded deployment{} still in place",
        findings.leftovers.len(),
        plural(findings.leftovers.len()),
        findings.present,
        match findings.present {
            1 => " is",
            _ => "s are",
        },
    ));
    text
}

fn plural(n: usize) -> &'static str {
    match n {
        1 => "",
        _ => "s",
    }
}
