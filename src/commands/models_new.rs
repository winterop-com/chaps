//! `varde models new`: create a model project from a template.
//!
//! The project is a directory of its own, outside any deployment, so the
//! command does not read `-C`. What the templates hold is in
//! [`crate::scaffold`].

use crate::cli::ModelsNewArgs;
use crate::commands::Ctx;
use crate::error::{ChapError, Result};
use crate::scaffold::{self, Names, Template};
use anyhow::Context;
use serde::Serialize;

/// What `models new` did, and the shape of its `--json`.
#[derive(Debug, Serialize)]
struct NewReport {
    dir: String,
    template: Template,
    #[serde(flatten)]
    names: Names,
    base_image: String,
    /// The files written, relative to `dir`.
    files: Vec<String>,
}

/// Create the project directory and write the files of the template.
pub fn run(ctx: &Ctx, args: &ModelsNewArgs) -> Result<()> {
    if args.with_validation && args.template.is_mlproject() {
        return Err(ChapError::Usage(format!(
            "--with-validation adds hooks to main.py, and the {} template has no main.py; \
             use a chapkit service type such as fn-py or shell-r",
            args.template.as_str()
        ))
        .into());
    }
    let dir = std::path::absolute(&args.dir)
        .with_context(|| format!("could not resolve {}", args.dir.display()))?;
    let dir_name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .with_context(|| format!("{} has no directory name", args.dir.display()))?;
    let names = scaffold::names(&dir_name)?;
    let files = scaffold::render(&names, args.template, args.with_validation)?;
    scaffold::write(&dir, &files)?;

    let shown = args.dir.display().to_string();
    let report = NewReport {
        dir: dir.display().to_string(),
        template: args.template,
        base_image: args.template.base_image(),
        files: files.iter().map(|(path, _)| path.to_string()).collect(),
        names,
    };
    ctx.out.report_ok(&report, |r| {
        r.info(format!(
            "created {} in {shown} ({} template, service id {})",
            report.names.name,
            args.template.as_str(),
            report.names.service_id
        ));
        r.info(format!(
            "next: build the image as `{shown}/README.md` says, then run `varde run {}:dev`",
            report.names.service_id
        ));
        r.hint(format!("files: {}", report.files.join(", ")));
        r.hint(format!("base image: {}", report.base_image));
    })
}
