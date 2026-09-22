//! `chaps` — generate and manage a docker compose deployment of chap-core plus
//! marketplace model services.

// Stubs owned by agents A, B and C are not called yet; remove after A/B/C land.

mod cli;
mod commands;
mod compose;
mod docker;
mod error;
mod output;
mod paths;
mod project;
mod registry;
mod status;
mod tui;

use clap::Parser;
use cli::{Cli, Command, DockerCmd, ModelsCmd};
use commands::Ctx;
use error::ChapError;

fn main() {
    let cli = Cli::parse();
    let ctx = Ctx::from_cli(&cli);

    if let Err(err) = dispatch(&ctx, &cli) {
        let rendered = ctx.out.error(&err);
        // JSON consumers read stdout, so a --json error goes there too.
        if ctx.out.json {
            println!("{rendered}");
        } else {
            eprintln!("{rendered}");
        }
        std::process::exit(exit_code(&err));
    }
}

fn dispatch(ctx: &Ctx, cli: &Cli) -> error::Result<()> {
    match &cli.command {
        Command::Init(args) => commands::init::run(ctx, args),

        Command::Models(m) => match &m.command {
            ModelsCmd::List(args) => commands::models::list(ctx, args),
            ModelsCmd::Search(args) => commands::models::search(ctx, args),
            ModelsCmd::Info(args) => commands::models::info(ctx, args),
            ModelsCmd::Enable(args) => commands::enable::enable(ctx, args),
            ModelsCmd::Disable(args) => commands::enable::disable(ctx, args),
            ModelsCmd::Tui(args) => run_tui(ctx, args),
        },
        Command::Tui(args) => run_tui(ctx, args),

        Command::Registry(r) => commands::registry::run(ctx, &r.command),

        Command::Sync(args) => commands::sync::run(ctx, args),
        Command::Update(args) => commands::update::run(ctx, args),

        Command::Up(args) => commands::docker::run(ctx, &DockerCmd::Up(args.clone())),
        Command::Down(args) => commands::docker::run(ctx, &DockerCmd::Down(args.clone())),
        Command::Ps(args) => commands::docker::run(ctx, &DockerCmd::Ps(args.clone())),
        Command::Logs(args) => commands::docker::run(ctx, &DockerCmd::Logs(args.clone())),
        Command::Pull(args) => commands::docker::run(ctx, &DockerCmd::Pull(args.clone())),
        Command::Compose(args) => commands::docker::run(ctx, &DockerCmd::Compose(args.clone())),

        Command::Status(args) => commands::status::run(ctx, args),
    }
}

/// The browser owns the terminal, so there is nothing to serialise: reject
/// `--json` rather than print something a caller cannot parse.
fn run_tui(ctx: &Ctx, args: &cli::TuiArgs) -> error::Result<()> {
    if ctx.out.json {
        return Err(anyhow::anyhow!(
            "--json cannot be combined with the model browser"
        ));
    }
    commands::tui::run(ctx, args)
}

/// Mirror docker compose's exit code when it is the thing that failed, so
/// `chaps up` is a drop-in for `docker compose up` in scripts.
fn exit_code(err: &anyhow::Error) -> i32 {
    match err.downcast_ref::<ChapError>() {
        Some(ChapError::DockerFailed(code)) if *code != 0 => *code,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_failures_keep_their_exit_code() {
        let err = anyhow::Error::new(ChapError::DockerFailed(137));
        assert_eq!(exit_code(&err), 137);
    }

    #[test]
    fn everything_else_exits_one() {
        assert_eq!(exit_code(&anyhow::anyhow!("boom")), 1);
        assert_eq!(
            exit_code(&anyhow::Error::new(ChapError::DockerFailed(0))),
            1
        );
        assert_eq!(
            exit_code(&anyhow::Error::new(ChapError::UnknownModel("x".into()))),
            1
        );
    }

    #[test]
    fn json_rejects_the_browser() {
        let cli = Cli::try_parse_from(["chap", "--json", "tui"]).unwrap();
        let ctx = Ctx::from_cli(&cli);
        let Command::Tui(args) = &cli.command else {
            panic!("expected tui");
        };
        let err = run_tui(&ctx, args).expect_err("--json with tui is rejected");
        assert!(err.to_string().contains("--json"));
    }
}
