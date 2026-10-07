//! `varde cleanup`: remove what deployments that no longer exist left in
//! docker - their volumes and their network.
//!
//! Only what varde can prove is left over goes. A deployment counts as gone
//! when varde recorded it ([`crate::known`]) and its directory no longer holds
//! it; a deployment that is only down still has its directory, and its data
//! stays. Volumes varde never recorded a deployment for are not touched, and
//! neither is anything a container still uses.

use crate::cli::CleanupArgs;
use crate::commands::Ctx;
use crate::docker;
use crate::error::Result;
use crate::output::Report;
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

/// What `varde cleanup` did, for `--json`.
#[derive(Debug, Serialize)]
struct CleanupReport {
    dry_run: bool,
    #[serde(flatten)]
    findings: Findings,
    removed_volumes: Vec<String>,
    removed_networks: Vec<String>,
    refused: Vec<Refusal>,
    /// What went wrong on the way that did not stop the run.
    #[serde(skip)]
    warnings: Vec<String>,
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
            "docker is not answering, so nothing was checked; start Docker, then `varde cleanup` \
             again"
        ));
    };
    let mut report = CleanupReport {
        dry_run: args.dry_run,
        findings,
        removed_volumes: Vec::new(),
        removed_networks: Vec::new(),
        refused: Vec::new(),
        warnings: Vec::new(),
    };
    if report.findings.leftovers.is_empty() {
        if !args.dry_run {
            crate::known::forget(&report.findings.forgotten)?;
        }
        return ctx.out.report_ok(&report, |lines| summary(&report, lines));
    }
    if !args.dry_run {
        if !args.yes {
            confirm(ctx, &report.findings)?;
        }
        remove(&mut report);
    }
    ctx.out.report_ok(&report, |lines| summary(&report, lines))
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
            // Asked again rather than assumed: docker refuses a network a
            // container is still attached to.
            match docker::default_network_exists(&leftover.project) {
                false => report.removed_networks.push(network.clone()),
                true => {
                    all_gone = false;
                    report.refused.push(Refusal {
                        volume: network.clone(),
                        why: "docker kept the network; a container is still attached to it"
                            .to_string(),
                    });
                }
            }
        }
        if all_gone {
            done.push(leftover.project.clone());
        }
    }
    if let Err(err) = crate::known::forget(&done) {
        report.warnings.push(format!("{err:#}"));
    }
}

/// Name what will go, and get a yes. Nobody to ask - `--json`, or no
/// terminal - is a refusal, as for `varde down --volumes`: there is no
/// undoing this.
fn confirm(ctx: &Ctx, findings: &Findings) -> Result<()> {
    if ctx.out.json {
        return Err(anyhow::anyhow!(
            "--json cannot ask before deleting data; `varde cleanup --dry-run` lists it, and \
             `varde cleanup --yes` deletes it"
        ));
    }
    if !std::io::stdin().is_terminal() {
        return Err(anyhow::anyhow!(
            "this deletes data and there is no terminal to confirm at; `varde cleanup --yes` \
             deletes it without asking"
        ));
    }
    for line in listing(findings) {
        eprintln!("{line}");
    }
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

/// Each gone deployment with what it left, one line each.
fn listing(findings: &Findings) -> Vec<String> {
    let mut lines = Vec::new();
    for leftover in &findings.leftovers {
        lines.push(format!(
            "{} (was {})",
            leftover.project,
            leftover.dir.display()
        ));
        for volume in &leftover.volumes {
            lines.push(format!("  volume {volume}"));
        }
        if let Some(network) = &leftover.network {
            lines.push(format!("  network {network}"));
        }
    }
    lines
}

/// `1 recorded deployment is still in place`, `2 ... are ...`.
fn in_place(present: usize) -> String {
    format!(
        "{present} recorded deployment{} still in place",
        match present {
            1 => " is",
            _ => "s are",
        }
    )
}

fn summary(report: &CleanupReport, lines: &mut Report) {
    let findings = &report.findings;
    for kept in &findings.kept {
        lines.warning(format!(
            "kept {} (was {}): {}",
            kept.project,
            kept.dir.display(),
            kept.reason
        ));
    }
    for warning in &report.warnings {
        lines.warning(warning.as_str());
    }
    if findings.leftovers.is_empty() {
        lines.info("nothing to clean up");
        match findings.forgotten.len() {
            0 => {}
            n if report.dry_run => {
                lines.hint(format!("{n} removed deployment{} left nothing", plural(n)));
            }
            n => {
                lines.hint(format!(
                    "forgot {n} removed deployment{} that left nothing",
                    plural(n)
                ));
            }
        }
        lines.hint(in_place(findings.present));
        return;
    }
    if report.dry_run {
        for line in listing(findings) {
            lines.info(line);
        }
        lines
            .info(format!(
                "would delete {} volume{} of {} removed deployment{}",
                findings.volume_count(),
                plural(findings.volume_count()),
                findings.leftovers.len(),
                plural(findings.leftovers.len())
            ))
            .hint("`varde cleanup` deletes them");
        return;
    }
    for volume in &report.removed_volumes {
        lines.info(format!("removed volume {volume}"));
    }
    for network in &report.removed_networks {
        lines.info(format!("removed network {network}"));
    }
    for refusal in &report.refused {
        lines.warning(format!(
            "kept {}: {}; `docker ps -a` shows what holds it",
            refusal.volume, refusal.why
        ));
    }
    lines
        .info(format!(
            "cleaned up {} removed deployment{}",
            findings.leftovers.len(),
            plural(findings.leftovers.len())
        ))
        .hint(in_place(findings.present));
}

fn plural(n: usize) -> &'static str {
    match n {
        1 => "",
        _ => "s",
    }
}
