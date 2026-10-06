//! `varde top`: the live tree on a terminal, one snapshot anywhere else.

use crate::cli::TopArgs;
use crate::commands::Ctx;
use crate::error::Result;
use crate::output::Out;
use crate::project::Project;
use crate::top::data::{self, Node};
use std::io::IsTerminal;
use std::path::PathBuf;

/// The deployments to show even with no container: the one the command runs
/// in, and every `varde run` group.
fn known(ctx: &Ctx) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = super::run::groups().into_iter().map(|(_, d)| d).collect();
    if let Some(here) = Project::find_root(&ctx.project_dir) {
        dirs.insert(0, here);
    }
    dirs
}

/// The screen on a terminal; under `--json`, or with stdout not a terminal,
/// one snapshot of the same tree.
pub fn run(ctx: &Ctx, args: &TopArgs) -> Result<()> {
    if ctx.out.json || !std::io::stdout().is_terminal() {
        let nodes = data::collect(&known(ctx));
        let value = serde_json::json!({ "deployments": nodes });
        return ctx.out.emit(&value, || snapshot(&nodes, &ctx.out));
    }
    crate::top::run(ctx, known(ctx), args.interval)
}

/// The tree as text, for a pipe.
fn snapshot(nodes: &[Node], out: &Out) -> String {
    if nodes.is_empty() {
        return out.backticks(
            "no varde deployment has a container on this machine; \
             `varde run <model>` or `varde up` starts one",
        );
    }
    let mut text = String::new();
    for node in nodes {
        text.push_str(&format!(
            "{}  {}  {}/{} running{}\n",
            node.name,
            node.kind.label(),
            node.running(),
            node.services.len(),
            node.dir
                .as_ref()
                .map(|d| format!("  {}", d.display()))
                .unwrap_or_default()
        ));
        let rows: Vec<Vec<String>> = node
            .services
            .iter()
            .map(|s| {
                vec![
                    format!("  {}", s.service),
                    match &s.health {
                        Some(h) => format!("{} ({h})", s.state),
                        None => s.state.clone(),
                    },
                    s.cpu.clone().unwrap_or_default(),
                    s.memory.clone().unwrap_or_default(),
                    s.url.clone().unwrap_or_default(),
                ]
            })
            .collect();
        text.push_str(&out.table(&["  SERVICE", "STATE", "CPU", "MEMORY", "URL"], &rows));
    }
    text.push_str(&out.dim("a snapshot: on a terminal `varde top` keeps it up to date"));
    text
}
