//! The port check `varde up` runs before docker is let near the ports.

use super::note;
use crate::commands::Ctx;
use crate::docker;
use crate::error::Result;
use crate::ports;
use crate::project::Project;
use std::io::{BufRead, IsTerminal, Write};

/// Refuse to start when a host port the stack publishes is already taken.
///
/// Docker would find the same conflict, several seconds in, and name a
/// container rather than a port; this says which port, who wanted it and how
/// to move it, before anything has started. Ports held by this project's own
/// running containers are ours, so they are skipped: `varde up` on a running
/// stack has to stay a no-op.
///
/// When every taken port is published by another varde deployment, that is
/// almost always the one to put away, so `up` offers to: at a terminal it asks,
/// with `--replace` it does it without asking, and otherwise it refuses and
/// names both ways.
pub(crate) fn preflight(ctx: &Ctx, project: &Project, replace: bool) -> Result<()> {
    let claims = ports::claims(project);
    if claims.is_empty() {
        return Ok(());
    }
    let running = docker::running_services(project);
    let busy = ports::busy_claims(&claims, &running, &ports::is_busy);
    if busy.is_empty() {
        return Ok(());
    }
    // Who else publishes each port, and a free port above each one, found the
    // way `init` finds them: a port another deployment publishes is as good
    // as taken when suggesting one.
    let others = ports::other_deployments(&project.dir, &docker::compose_ls_json);
    let taken = |port: u16| ports::is_busy(port) || others.iter().any(|other| other.holds(port));
    let conflicts: Vec<ports::Conflict> = busy
        .into_iter()
        .map(|claim| ports::Conflict {
            suggestion: ports::first_free(claim.port.saturating_add(1), u16::MAX, &taken),
            holders: others
                .iter()
                .filter(|other| other.holds(claim.port))
                .collect(),
            claim,
        })
        .collect();
    let message = ports::preflight_message(&conflicts);
    // Only when every conflict is another varde deployment: a port some other
    // program holds is not varde's to take away.
    if conflicts.iter().any(|conflict| conflict.holders.is_empty()) {
        return Err(anyhow::anyhow!(message));
    }
    let mut holders: Vec<&ports::Deployment> = Vec::new();
    for conflict in &conflicts {
        for held in &conflict.holders {
            if !holders.iter().any(|seen| seen.dir == held.dir) {
                holders.push(held);
            }
        }
    }
    let names = holders
        .iter()
        .map(|held| held.name())
        .collect::<Vec<_>>()
        .join(", ");
    if !replace {
        let interactive = !ctx.out.json && std::io::stdin().is_terminal();
        if !interactive {
            return Err(anyhow::anyhow!(
                "{message}\n  or run `varde up --replace` to stop {names} first"
            ));
        }
        eprintln!("{message}");
        eprint!(
            "\n{names} {} using these ports. Stop {} and start this deployment instead? \
             Its data is kept. [y/N] ",
            if holders.len() == 1 { "is" } else { "are" },
            if holders.len() == 1 { "it" } else { "them" },
        );
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut answer)
            .map_err(|e| anyhow::anyhow!("reading the answer: {e}"))?;
        if !matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
            return Err(anyhow::anyhow!("cancelled; nothing was started or stopped"));
        }
    }
    for held in &holders {
        let other = Project::load(&held.dir)?;
        note(
            ctx,
            &format!(
                "stopping {} ({}) to free its ports; `varde -C {} up` starts it again",
                held.name(),
                held.dir.display(),
                held.dir.display()
            ),
        );
        docker::run_compose(&other, &["down".to_string()])?;
    }
    // What stopped them is what is asked again: a port something else took
    // in the meantime is the refusal it always was.
    let busy = ports::busy_claims(&claims, &running, &ports::is_busy);
    if !busy.is_empty() {
        let still: Vec<String> = busy.iter().map(|claim| claim.port.to_string()).collect();
        return Err(anyhow::anyhow!(
            "stopped {names}, and port {} is still in use; run `varde up` again to see by what",
            still.join(", ")
        ));
    }
    Ok(())
}
